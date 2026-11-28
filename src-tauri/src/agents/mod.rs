pub(crate) mod instructions;
pub(crate) mod remove;
mod store;
pub(crate) mod validate;
pub(crate) mod workspace;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::auth::{self, AppAuth, AuthMode};
use crate::db::Database;
use crate::providers;
use crate::secrets::SharedStore;

pub const WORKSPACES_DIR_ENV: &str = "SHIKIGAMI_WORKSPACES_DIR";

pub struct WorkspacesRoot(pub Result<PathBuf, String>);

pub fn resolve_workspaces_root(
    home: Option<PathBuf>,
    override_dir: Option<OsString>,
) -> Result<PathBuf, String> {
    match override_dir {
        Some(dir) if !dir.is_empty() => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                Ok(path)
            } else {
                Err(format!(
                    "{WORKSPACES_DIR_ENV} must be an absolute path, but it is {}",
                    path.display()
                ))
            }
        }
        _ => home
            .filter(|home| home.is_absolute())
            .map(|home| home.join("Shikigami").join("agents"))
            .ok_or_else(|| "could not find your home folder".to_string()),
    }
}

#[derive(Debug, Serialize)]
pub struct AgentError {
    pub code: &'static str,
    pub message: String,
}

impl AgentError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn name_taken() -> Self {
        Self::new("name-taken", "An agent with this name already exists.")
    }

    pub fn database(error: impl std::fmt::Display) -> Self {
        Self::new(
            "database-failed",
            format!("Could not save the agent: {error}"),
        )
    }

    fn folder(error: impl std::fmt::Display) -> Self {
        Self::new(
            "folder-failed",
            format!("Could not make the agent's folder: {error}"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentState {
    Idle,
    Working,
    NeedsAttention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attention {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub color: String,
    pub provider: String,
    pub model: String,
    pub workspace_path: String,
    pub created_at: i64,
    pub last_activity_at: i64,
    pub state: AgentState,
    pub attention: Option<Attention>,
    pub auth_mode: Option<AuthMode>,
    pub effective_auth_mode: AuthMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentInput {
    pub name: String,
    pub color: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub auth_mode: Option<AuthMode>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

struct Validated {
    name: String,
    color: String,
    provider: String,
    model: String,
    auth_mode: Option<AuthMode>,
}

struct Facts<'a> {
    has_running_turn: bool,
    newest_turn_status: Option<&'a str>,
    newest_turn_error: Option<&'a str>,
    folder_exists: bool,
    provider_installed: bool,
    provider_name: &'a str,
}

fn derive_attention(facts: &Facts) -> Option<Attention> {
    if facts.has_running_turn {
        return None;
    }
    if !facts.folder_exists {
        return Some(Attention {
            code: "workspace-missing",
            message: "The workspace folder is missing.".to_owned(),
        });
    }
    if !facts.provider_installed {
        return Some(Attention {
            code: "provider-unavailable",
            message: format!("{} is not installed.", facts.provider_name),
        });
    }
    match facts.newest_turn_status {
        Some("interrupted") => Some(Attention {
            code: "last-turn-interrupted",
            message: "The app closed while the agent was working.".to_owned(),
        }),
        Some("failed") => Some(Attention {
            code: "last-turn-failed",
            message: match facts.newest_turn_error {
                Some(error) if !error.trim().is_empty() => {
                    format!("The last reply failed: {error}")
                }
                _ => "The last reply failed.".to_owned(),
            },
        }),
        _ => None,
    }
}

fn derive_state(facts: &Facts) -> (AgentState, Option<Attention>) {
    if facts.has_running_turn {
        return (AgentState::Working, None);
    }
    match derive_attention(facts) {
        Some(attention) => (AgentState::NeedsAttention, Some(attention)),
        None => (AgentState::Idle, None),
    }
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

fn validate(
    db: &Database,
    input: CreateAgentInput,
    dirs: &[PathBuf],
) -> Result<Validated, AgentError> {
    let name = validate::name(&input.name)?;
    let color = validate::color(&input.color)?;
    if !providers::is_known(&input.provider) || !providers::is_installed(&input.provider, dirs) {
        return Err(AgentError::new(
            "provider-unavailable",
            "This provider is not installed.",
        ));
    }
    let model = validate::model(&input.model)?;
    let taken = db
        .with_connection(|connection| store::name_exists(connection, &name))
        .map_err(AgentError::database)?;
    if taken {
        return Err(AgentError::name_taken());
    }
    Ok(Validated {
        name,
        color,
        provider: input.provider,
        model,
        auth_mode: input.auth_mode,
    })
}

pub(crate) fn recorded_workspaces(db: &Database) -> rusqlite::Result<Vec<PathBuf>> {
    db.with_connection(store::workspace_paths)
        .map(|paths| paths.into_iter().map(PathBuf::from).collect())
}

fn persist(
    db: &Database,
    root: &Path,
    valid: Validated,
    app: &AppAuth,
) -> Result<Agent, AgentError> {
    let recorded: Vec<PathBuf> = db
        .with_connection(store::workspace_paths)
        .map_err(AgentError::database)?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let folder =
        workspace::create_folder(root, &valid.name, &recorded).map_err(AgentError::folder)?;
    let now = now_millis();
    let agent = Agent {
        id: uuid::Uuid::now_v7().to_string(),
        name: valid.name,
        color: valid.color,
        provider: valid.provider,
        model: valid.model,
        workspace_path: folder.to_string_lossy().into_owned(),
        created_at: now,
        last_activity_at: now,
        state: AgentState::Idle,
        attention: None,
        auth_mode: valid.auth_mode,
        effective_auth_mode: app.effective(valid.auth_mode),
    };
    let saved = workspace::write_instruction_file(&folder, &agent.name)
        .map_err(AgentError::folder)
        .and_then(|()| db.with_connection(|connection| store::insert(connection, &agent)));
    match saved {
        Ok(()) => Ok(agent),
        Err(error) => Err(discard_folder(&folder, error)),
    }
}

fn discard_folder(folder: &Path, error: AgentError) -> AgentError {
    match fs::remove_dir_all(folder) {
        Err(failure) if failure.kind() != std::io::ErrorKind::NotFound => AgentError::new(
            error.code,
            format!(
                "{} The folder {} could not be removed.",
                error.message,
                folder.display()
            ),
        ),
        _ => error,
    }
}

fn create(
    db: &Database,
    root: &Result<PathBuf, String>,
    input: CreateAgentInput,
    dirs: &[PathBuf],
    app: &AppAuth,
) -> Result<Agent, AgentError> {
    let valid = validate(db, input, dirs)?;
    let root = root
        .as_ref()
        .map_err(|message| AgentError::new("folder-failed", message.clone()))?;
    persist(db, root, valid, app)
}

fn list(db: &Database, dirs: &[PathBuf], app: &AppAuth) -> Result<Vec<Agent>, AgentError> {
    let rows = db
        .with_connection(store::list)
        .map_err(AgentError::database)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let (state, attention) = derive_state(&Facts {
                has_running_turn: row.has_running_turn,
                newest_turn_status: row.newest_turn_status.as_deref(),
                newest_turn_error: row.newest_turn_error.as_deref(),
                folder_exists: Path::new(&row.workspace_path).is_dir(),
                provider_installed: providers::is_installed(&row.provider, dirs),
                provider_name: providers::name_of(&row.provider),
            });
            let auth_mode = row.auth_mode.as_deref().and_then(AuthMode::parse);
            Agent {
                id: row.id,
                name: row.name,
                color: row.color,
                provider: row.provider,
                model: row.model,
                workspace_path: row.workspace_path,
                created_at: row.created_at,
                last_activity_at: row.last_activity_at,
                state,
                attention,
                auth_mode,
                effective_auth_mode: app.effective(auth_mode),
            }
        })
        .collect())
}

fn recreate(
    db: &Database,
    dirs: &[PathBuf],
    app: &AppAuth,
    agent_id: &str,
) -> Result<Agent, AgentError> {
    let (name, path) = db
        .with_connection(|connection| store::workspace_of(connection, agent_id))
        .map_err(AgentError::database)?
        .ok_or_else(|| AgentError::new("not-found", "This agent no longer exists."))?;
    let shared = db
        .with_connection(|connection| store::path_used_by_other(connection, agent_id, &path))
        .map_err(AgentError::database)?;
    if shared {
        return Err(AgentError::new(
            "folder-in-use",
            remove::SHARED_FOLDER_MESSAGE,
        ));
    }
    let folder = PathBuf::from(path);
    fs::create_dir_all(&folder).map_err(AgentError::folder)?;
    match workspace::write_instruction_file(&folder, &name) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
            return Err(AgentError::folder(error));
        }
        _ => {}
    }
    find(db, dirs, app, agent_id)
}

fn find(
    db: &Database,
    dirs: &[PathBuf],
    app: &AppAuth,
    agent_id: &str,
) -> Result<Agent, AgentError> {
    list(db, dirs, app)?
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| AgentError::new("not-found", "This agent no longer exists."))
}

fn set_auth_mode(
    db: &Database,
    dirs: &[PathBuf],
    app: &AppAuth,
    agent_id: &str,
    mode: Option<AuthMode>,
) -> Result<Agent, AgentError> {
    let changed = db
        .with_connection(|connection| store::set_auth_mode(connection, agent_id, mode))
        .map_err(AgentError::database)?;
    if changed == 0 {
        return Err(AgentError::new("not-found", "This agent no longer exists."));
    }
    find(db, dirs, app, agent_id)
}

fn update(
    db: &Database,
    dirs: &[PathBuf],
    app: &AppAuth,
    agent_id: &str,
    input: UpdateAgentInput,
) -> Result<Agent, AgentError> {
    let name = input.name.as_deref().map(validate::name).transpose()?;
    let color = input.color.as_deref().map(validate::color).transpose()?;
    let model = input.model.as_deref().map(validate::model).transpose()?;
    let changed = db.with_connection(|connection| {
        store::update(
            connection,
            agent_id,
            name.as_deref(),
            color.as_deref(),
            model.as_deref(),
        )
    })?;
    if changed == 0 {
        return Err(AgentError::new("not-found", "This agent no longer exists."));
    }
    find(db, dirs, app, agent_id)
}

fn folder_of(db: &Database, agent_id: &str) -> Result<PathBuf, AgentError> {
    db.with_connection(|connection| store::workspace_of(connection, agent_id))
        .map_err(AgentError::database)?
        .map(|(_, path)| PathBuf::from(path))
        .ok_or_else(|| AgentError::new("not-found", "This agent no longer exists."))
}

#[cfg(not(feature = "e2e"))]
fn reveal_folder(folder: &Path) -> Result<(), AgentError> {
    tauri_plugin_opener::reveal_item_in_dir(folder).map_err(|error| {
        AgentError::new(
            "folder-failed",
            format!("Could not show the folder in Finder: {error}"),
        )
    })
}

#[cfg(feature = "e2e")]
fn reveal_folder(folder: &Path) -> Result<(), AgentError> {
    use std::io::Write;
    if let Some(log) = std::env::var_os("SHIKIGAMI_E2E_REVEAL_LOG").filter(|log| !log.is_empty()) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map_err(AgentError::folder)?;
        writeln!(file, "{}", folder.display()).map_err(AgentError::folder)?;
    }
    Ok(())
}

fn reveal(db: &Database, agent_id: &str) -> Result<(), AgentError> {
    let folder = folder_of(db, agent_id)?;
    if !folder.is_dir() {
        return Err(AgentError::new(
            "folder-failed",
            "The workspace folder is missing.",
        ));
    }
    reveal_folder(&folder)
}

fn unavailable() -> AgentError {
    AgentError::new("unavailable", "The app database is not available.")
}

fn app_auth(app: &AppHandle, db: &Database) -> Result<AppAuth, AgentError> {
    let store = app.try_state::<SharedStore>().ok_or_else(unavailable)?;
    auth::load_app_auth(db, &**store).map_err(|error| AgentError::new(error.code, error.message))
}

#[tauri::command]
pub async fn list_agents(app: AppHandle) -> Result<Vec<Agent>, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let app_auth = app_auth(&app, &db)?;
        list(&db, &providers::search_dirs(), &app_auth)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn create_agent(app: AppHandle, input: CreateAgentInput) -> Result<Agent, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let root = app.try_state::<WorkspacesRoot>().ok_or_else(unavailable)?;
        let app_auth = app_auth(&app, &db)?;
        create(&db, &root.0, input, &providers::search_dirs(), &app_auth)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn recreate_workspace(app: AppHandle, agent_id: String) -> Result<Agent, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let app_auth = app_auth(&app, &db)?;
        recreate(&db, &providers::search_dirs(), &app_auth, &agent_id)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn set_agent_auth_mode(
    app: AppHandle,
    agent_id: String,
    mode: Option<AuthMode>,
) -> Result<Agent, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let app_auth = app_auth(&app, &db)?;
        set_auth_mode(&db, &providers::search_dirs(), &app_auth, &agent_id, mode)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn update_agent(
    app: AppHandle,
    agent_id: String,
    input: UpdateAgentInput,
) -> Result<Agent, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let app_auth = app_auth(&app, &db)?;
        update(&db, &providers::search_dirs(), &app_auth, &agent_id, input)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn read_instructions(
    app: AppHandle,
    agent_id: String,
) -> Result<instructions::Instructions, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        instructions::read(&folder_of(&db, &agent_id)?)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn write_instructions(
    app: AppHandle,
    agent_id: String,
    text: String,
    expected_modified_at: Option<i64>,
    force: bool,
) -> Result<instructions::Written, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        instructions::write(
            &folder_of(&db, &agent_id)?,
            &text,
            expected_modified_at,
            force,
        )
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn reveal_workspace(app: AppHandle, agent_id: String) -> Result<(), AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        reveal(&db, &agent_id)
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn delete_agent(
    app: AppHandle,
    agent_id: String,
    move_folder_to_trash: bool,
) -> Result<remove::Deleted, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let root = app.try_state::<WorkspacesRoot>().ok_or_else(unavailable)?;
        remove::delete(
            &db,
            &root.0,
            &agent_id,
            move_folder_to_trash,
            &remove::system_trash,
        )
    })
    .await
    .map_err(AgentError::database)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const APP_AUTH: AppAuth = AppAuth {
        setting: None,
        key_exists: false,
    };

    struct Fixture {
        _data: tempfile::TempDir,
        workspaces: tempfile::TempDir,
        bin: tempfile::TempDir,
        db: Database,
    }

    impl Fixture {
        fn new(clis: &[&str]) -> Self {
            let data = tempfile::tempdir().unwrap();
            let bin = tempfile::tempdir().unwrap();
            for cli in clis {
                let path = bin.path().join(cli);
                fs::write(&path, "#!/bin/sh\n").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let db = Database::open(data.path()).unwrap();
            Self {
                _data: data,
                workspaces: tempfile::tempdir().unwrap(),
                bin,
                db,
            }
        }

        fn dirs(&self) -> Vec<PathBuf> {
            vec![self.bin.path().to_path_buf()]
        }

        fn root(&self) -> Result<PathBuf, String> {
            Ok(self.workspaces.path().to_path_buf())
        }

        fn create(&self, name: &str, provider: &str) -> Result<Agent, AgentError> {
            create(
                &self.db,
                &self.root(),
                input(name, provider),
                &self.dirs(),
                &APP_AUTH,
            )
        }

        fn list(&self) -> Vec<Agent> {
            list(&self.db, &self.dirs(), &APP_AUTH).unwrap()
        }

        fn agent_count(&self) -> i64 {
            self.db.with_connection(|c| {
                c.query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
                    .unwrap()
            })
        }

        fn folder_count(&self) -> usize {
            fs::read_dir(self.workspaces.path()).unwrap().count()
        }
    }

    fn input(name: &str, provider: &str) -> CreateAgentInput {
        CreateAgentInput {
            name: name.to_string(),
            color: "blue".to_string(),
            provider: provider.to_string(),
            model: "default".to_string(),
            auth_mode: None,
        }
    }

    fn error_code(result: Result<Agent, AgentError>) -> &'static str {
        result.expect_err("expected an error").code
    }

    #[test]
    fn create_makes_the_folder_and_the_claude_instruction_file() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let folder = fx.workspaces.path().join("test-agent");
        assert_eq!(agent.workspace_path, folder.to_string_lossy());
        assert_eq!(
            fs::read_to_string(folder.join("CLAUDE.md")).unwrap(),
            "# Test Agent\n\nYou are Test Agent, a coding agent working in this folder.\n"
        );
        assert!(!folder.join("AGENTS.md").exists());
        assert_eq!(agent.state, AgentState::Idle);
        assert_eq!(fx.list(), vec![agent]);
    }

    #[test]
    fn names_that_share_a_folder_name_get_numbered_folders() {
        let fx = Fixture::new(&["claude"]);
        fx.create("Test Agent", "claude-code").unwrap();
        let second = fx.create("test-agent", "claude-code").unwrap();
        assert!(second.workspace_path.ends_with("test-agent-2"));
    }

    #[test]
    fn a_duplicate_name_is_refused_ignoring_case() {
        let fx = Fixture::new(&["claude"]);
        fx.create("Test Agent", "claude-code").unwrap();
        assert_eq!(
            error_code(fx.create("test agent", "claude-code")),
            "name-taken"
        );
        assert_eq!(
            error_code(fx.create("  TEST AGENT  ", "claude-code")),
            "name-taken"
        );
        assert_eq!(fx.agent_count(), 1);
        assert_eq!(fx.folder_count(), 1);
    }

    #[test]
    fn each_rule_has_its_own_error_code() {
        let fx = Fixture::new(&["claude"]);
        assert_eq!(error_code(fx.create("  ", "claude-code")), "name-empty");
        assert_eq!(
            error_code(fx.create(&"a".repeat(41), "claude-code")),
            "name-too-long"
        );
        assert_eq!(
            error_code(fx.create("Line\nbreak", "claude-code")),
            "name-invalid"
        );
        assert_eq!(
            error_code(fx.create("A", "unknown")),
            "provider-unavailable"
        );
        let mut bad_color = input("A", "claude-code");
        bad_color.color = "purple".to_string();
        assert_eq!(
            error_code(create(&fx.db, &fx.root(), bad_color, &fx.dirs(), &APP_AUTH)),
            "color-invalid"
        );
        let mut bad_model = input("A", "claude-code");
        bad_model.model = "two words".to_string();
        assert_eq!(
            error_code(create(&fx.db, &fx.root(), bad_model, &fx.dirs(), &APP_AUTH)),
            "model-invalid"
        );
        assert_eq!(fx.agent_count(), 0);
        assert_eq!(fx.folder_count(), 0);
    }

    #[test]
    fn a_failed_insert_removes_the_new_folder_and_leaves_no_row() {
        let fx = Fixture::new(&["claude"]);
        fx.create("Taken", "claude-code").unwrap();
        let valid = Validated {
            name: "taken".to_string(),
            color: "blue".to_string(),
            provider: "claude-code".to_string(),
            model: "default".to_string(),
            auth_mode: None,
        };
        let error = persist(&fx.db, fx.workspaces.path(), valid, &APP_AUTH).unwrap_err();
        assert_eq!(error.code, "name-taken");
        assert_eq!(fx.agent_count(), 1);
        let folders: Vec<_> = fs::read_dir(fx.workspaces.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(folders, vec![std::ffi::OsString::from("taken")]);
    }

    fn sample_agent(id: &str, name: &str, provider: &str) -> Agent {
        Agent {
            id: id.to_string(),
            name: name.to_string(),
            color: "blue".to_string(),
            provider: provider.to_string(),
            model: "default".to_string(),
            workspace_path: format!("/tmp/{id}"),
            created_at: 1,
            last_activity_at: 1,
            state: AgentState::Idle,
            attention: None,
            auth_mode: None,
            effective_auth_mode: AuthMode::Subscription,
        }
    }

    #[test]
    fn only_a_unique_violation_on_the_name_maps_to_name_taken() {
        let fx = Fixture::new(&["claude"]);
        let insert = |agent: Agent| fx.db.with_connection(|c| store::insert(c, &agent));
        insert(sample_agent("a", "Ada", "claude-code")).unwrap();
        let same_name = insert(sample_agent("b", "ADA", "claude-code")).unwrap_err();
        assert_eq!(same_name.code, "name-taken");
        let same_id = insert(sample_agent("a", "Other", "claude-code")).unwrap_err();
        assert_eq!(same_id.code, "database-failed");
        let bad_provider = insert(sample_agent("c", "Third", "unknown")).unwrap_err();
        assert_eq!(bad_provider.code, "database-failed");
        insert(sample_agent("d", "Fourth", "claude-code")).unwrap();
        let rename = fx
            .db
            .with_connection(|c| store::update(c, "d", Some("ada"), None, None))
            .unwrap_err();
        assert_eq!(rename.code, "name-taken");
        let bad_model = fx
            .db
            .with_connection(|c| store::update(c, "d", None, None, Some("ok")));
        assert_eq!(bad_model.unwrap(), 1);
    }

    #[test]
    fn a_unique_violation_on_another_column_and_a_null_name_are_not_name_taken() {
        let fx = Fixture::new(&["claude"]);
        fx.db.with_connection(|c| {
            c.execute(
                "CREATE UNIQUE INDEX test_path ON agents(workspace_path)",
                [],
            )
            .unwrap()
        });
        let insert = |agent: Agent| fx.db.with_connection(|c| store::insert(c, &agent));
        insert(sample_agent("a", "Ada", "claude-code")).unwrap();
        let mut same_path = sample_agent("b", "Bob", "claude-code");
        same_path.workspace_path = "/tmp/a".to_string();
        assert_eq!(insert(same_path).unwrap_err().code, "database-failed");
        let null_name = fx
            .db
            .with_connection(|c| {
                c.execute(
                    "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                     VALUES ('n', NULL, 'blue', 'claude-code', 'default', '/tmp/n', 1, 1)",
                    [],
                )
            })
            .unwrap_err();
        assert!(null_name.to_string().contains("agents.name"));
        assert_eq!(store::save_error(null_name).code, "database-failed");
    }

    #[test]
    fn a_rollback_that_cannot_remove_the_folder_says_so_and_keeps_the_error_code() {
        let fx = Fixture::new(&["claude"]);
        let folder = fx.workspaces.path().join("stuck");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(fx.workspaces.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let error = discard_folder(&folder, AgentError::name_taken());
        fs::set_permissions(fx.workspaces.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(error.code, "name-taken");
        assert_eq!(
            error.message,
            format!(
                "An agent with this name already exists. The folder {} could not be removed.",
                folder.display()
            )
        );
        assert!(folder.exists());
    }

    #[test]
    fn a_rollback_that_removes_the_folder_leaves_the_message_unchanged() {
        let fx = Fixture::new(&["claude"]);
        let folder = fx.workspaces.path().join("gone");
        fs::create_dir(&folder).unwrap();
        let error = discard_folder(&folder, AgentError::name_taken());
        assert_eq!(error.message, "An agent with this name already exists.");
        assert!(!folder.exists());
        let missing = discard_folder(&folder, AgentError::name_taken());
        assert_eq!(missing.message, "An agent with this name already exists.");
    }

    #[test]
    fn a_folder_that_cannot_be_made_leaves_no_agent() {
        let fx = Fixture::new(&["claude"]);
        let blocker = fx.workspaces.path().join("blocker");
        fs::write(&blocker, "a file, not a folder").unwrap();
        let root = Ok(blocker);
        let error = create(
            &fx.db,
            &root,
            input("A", "claude-code"),
            &fx.dirs(),
            &APP_AUTH,
        )
        .unwrap_err();
        assert_eq!(error.code, "folder-failed");
        assert_eq!(fx.agent_count(), 0);
    }

    #[test]
    fn an_unresolved_root_is_an_error_not_a_panic() {
        let fx = Fixture::new(&["claude"]);
        let root = Err("could not find your home folder".to_string());
        let error = create(
            &fx.db,
            &root,
            input("A", "claude-code"),
            &fx.dirs(),
            &APP_AUTH,
        )
        .unwrap_err();
        assert_eq!(error.code, "folder-failed");
        assert_eq!(fx.agent_count(), 0);
    }

    fn facts() -> Facts<'static> {
        Facts {
            has_running_turn: false,
            newest_turn_status: None,
            newest_turn_error: None,
            folder_exists: true,
            provider_installed: true,
            provider_name: "Claude Code",
        }
    }

    fn state_of(facts: Facts) -> AgentState {
        derive_state(&facts).0
    }

    fn code_of(facts: Facts) -> Option<&'static str> {
        derive_state(&facts).1.map(|attention| attention.code)
    }

    #[test]
    fn state_follows_the_turn_rules() {
        assert_eq!(state_of(facts()), AgentState::Idle);
        for status in ["completed", "stopped"] {
            let settled = Facts {
                newest_turn_status: Some(status),
                ..facts()
            };
            assert_eq!(state_of(settled), AgentState::Idle);
        }
        let running = Facts {
            has_running_turn: true,
            ..facts()
        };
        assert_eq!(state_of(running), AgentState::Working);
        let running_but_broken = Facts {
            has_running_turn: true,
            newest_turn_status: Some("failed"),
            folder_exists: false,
            provider_installed: false,
            ..facts()
        };
        assert_eq!(
            derive_state(&running_but_broken),
            (AgentState::Working, None)
        );
        for status in ["failed", "interrupted"] {
            let broken = Facts {
                newest_turn_status: Some(status),
                ..facts()
            };
            assert_eq!(state_of(broken), AgentState::NeedsAttention);
        }
        let no_folder = Facts {
            folder_exists: false,
            ..facts()
        };
        assert_eq!(state_of(no_folder), AgentState::NeedsAttention);
        let no_provider = Facts {
            provider_installed: false,
            ..facts()
        };
        assert_eq!(state_of(no_provider), AgentState::NeedsAttention);
    }

    #[test]
    fn each_attention_reason_has_its_code_and_message() {
        let missing = derive_attention(&Facts {
            folder_exists: false,
            ..facts()
        })
        .unwrap();
        assert_eq!(missing.code, "workspace-missing");
        assert_eq!(missing.message, "The workspace folder is missing.");
        let unavailable = derive_attention(&Facts {
            provider_installed: false,
            ..facts()
        })
        .unwrap();
        assert_eq!(unavailable.code, "provider-unavailable");
        assert_eq!(unavailable.message, "Claude Code is not installed.");
        let interrupted = derive_attention(&Facts {
            newest_turn_status: Some("interrupted"),
            ..facts()
        })
        .unwrap();
        assert_eq!(interrupted.code, "last-turn-interrupted");
        assert_eq!(
            interrupted.message,
            "The app closed while the agent was working."
        );
        let failed = derive_attention(&Facts {
            newest_turn_status: Some("failed"),
            newest_turn_error: Some("boom"),
            ..facts()
        })
        .unwrap();
        assert_eq!(failed.code, "last-turn-failed");
        assert_eq!(failed.message, "The last reply failed: boom");
        assert_eq!(derive_attention(&facts()), None);
    }

    #[test]
    fn attention_reasons_are_checked_in_priority_order() {
        let everything = Facts {
            newest_turn_status: Some("failed"),
            newest_turn_error: Some("boom"),
            folder_exists: false,
            provider_installed: false,
            ..facts()
        };
        assert_eq!(code_of(everything), Some("workspace-missing"));
        let no_provider_and_failed = Facts {
            newest_turn_status: Some("failed"),
            provider_installed: false,
            ..facts()
        };
        assert_eq!(
            code_of(no_provider_and_failed),
            Some("provider-unavailable")
        );
        let interrupted_and_failed_text = Facts {
            newest_turn_status: Some("interrupted"),
            newest_turn_error: Some("boom"),
            ..facts()
        };
        assert_eq!(
            code_of(interrupted_and_failed_text),
            Some("last-turn-interrupted")
        );
    }

    fn insert_turn(fx: &Fixture, id: &str, agent_id: &str, status: &str, started_at: i64) {
        fx.db.with_connection(|c| {
            c.execute(
                "INSERT INTO turns (id, agent_id, status, started_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, agent_id, status, started_at],
            )
            .unwrap()
        });
    }

    #[test]
    fn listed_state_reads_turns_folder_and_provider() {
        let fx = Fixture::new(&["claude"]);
        let running = fx.create("Running", "claude-code").unwrap();
        let failed = fx.create("Failed", "claude-code").unwrap();
        let recovered = fx.create("Recovered", "claude-code").unwrap();
        let gone = fx.create("Gone", "claude-code").unwrap();
        insert_turn(&fx, "t1", &running.id, "running", 10);
        insert_turn(&fx, "t2", &failed.id, "completed", 10);
        insert_turn(&fx, "t3", &failed.id, "failed", 20);
        insert_turn(&fx, "t4", &recovered.id, "failed", 10);
        insert_turn(&fx, "t5", &recovered.id, "completed", 20);
        fs::remove_dir_all(&gone.workspace_path).unwrap();
        let states: std::collections::HashMap<_, _> = fx
            .list()
            .into_iter()
            .map(|agent| (agent.name, agent.state))
            .collect();
        assert_eq!(states["Running"], AgentState::Working);
        assert_eq!(states["Failed"], AgentState::NeedsAttention);
        assert_eq!(states["Recovered"], AgentState::Idle);
        assert_eq!(states["Gone"], AgentState::NeedsAttention);

        let without_cli = list(&fx.db, &[], &APP_AUTH).unwrap();
        let recovered = without_cli
            .iter()
            .find(|agent| agent.name == "Recovered")
            .unwrap();
        assert_eq!(recovered.state, AgentState::NeedsAttention);
    }

    #[test]
    fn an_agent_row_from_another_provider_needs_attention() {
        let fx = Fixture::new(&["claude"]);
        fx.db.with_connection(|c| {
            c.execute(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('old', 'Old', 'red', 'codex', 'default', ?1, 1, 1)",
                [fx.workspaces.path().to_string_lossy().as_ref()],
            )
            .unwrap()
        });
        let agent = fx.list().into_iter().next().unwrap();
        assert_eq!(agent.state, AgentState::NeedsAttention);
        assert_eq!(agent.attention.unwrap().code, "provider-unavailable");
    }

    #[test]
    fn list_is_ordered_by_last_activity_then_creation() {
        let fx = Fixture::new(&["claude"]);
        let a = fx.create("A", "claude-code").unwrap();
        let b = fx.create("B", "claude-code").unwrap();
        let c = fx.create("C", "claude-code").unwrap();
        fx.db.with_connection(|conn| {
            for (id, last, created) in [(&a.id, 300, 1), (&b.id, 100, 2), (&c.id, 100, 3)] {
                conn.execute(
                    "UPDATE agents SET last_activity_at = ?2, created_at = ?3 WHERE id = ?1",
                    rusqlite::params![id, last, created],
                )
                .unwrap();
            }
        });
        let names: Vec<_> = fx.list().into_iter().map(|agent| agent.name).collect();
        assert_eq!(names, ["A", "C", "B"]);
    }

    #[test]
    fn agents_survive_reopening_the_database() {
        let data = tempfile::tempdir().unwrap();
        let workspaces = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let claude = bin.path().join("claude");
        fs::write(&claude, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = vec![bin.path().to_path_buf()];
        let root = Ok(workspaces.path().to_path_buf());
        let db = Database::open(data.path()).unwrap();
        create(
            &db,
            &root,
            input("Persisted", "claude-code"),
            &dirs,
            &APP_AUTH,
        )
        .unwrap();
        drop(db);
        let db = Database::open(data.path()).unwrap();
        let agents = list(&db, &dirs, &APP_AUTH).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "Persisted");
    }

    #[test]
    fn agent_serialises_in_camel_case_with_a_kebab_state() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Wire", "claude-code").unwrap();
        let value = serde_json::to_value(&agent).unwrap();
        assert!(value["workspacePath"].is_string());
        assert!(value["lastActivityAt"].is_i64());
        assert_eq!(value["state"], "idle");
        assert!(value["attention"].is_null());
        assert_eq!(
            serde_json::to_value(AgentState::NeedsAttention).unwrap(),
            "needs-attention"
        );
        let error = serde_json::to_value(AgentError::name_taken()).unwrap();
        assert_eq!(error["code"], "name-taken");
        assert!(error["message"].is_string());
    }

    #[test]
    fn workspaces_root_defaults_to_home_and_rejects_relative_overrides() {
        let home = || Some(PathBuf::from("/Users/me"));
        assert_eq!(
            resolve_workspaces_root(home(), None).unwrap(),
            PathBuf::from("/Users/me/Shikigami/agents")
        );
        assert_eq!(
            resolve_workspaces_root(home(), Some(OsString::new())).unwrap(),
            PathBuf::from("/Users/me/Shikigami/agents")
        );
        assert!(resolve_workspaces_root(None, None).is_err());
        assert!(resolve_workspaces_root(home(), Some("relative/dir".into())).is_err());
        assert_eq!(
            resolve_workspaces_root(None, Some("/tmp/elsewhere".into())).unwrap(),
            PathBuf::from("/tmp/elsewhere")
        );
    }

    #[test]
    fn a_listed_agent_carries_the_reason_it_needs_attention() {
        let fx = Fixture::new(&["claude"]);
        let failed = fx.create("Failed", "claude-code").unwrap();
        let gone = fx.create("Gone", "claude-code").unwrap();
        insert_turn(&fx, "t1", &failed.id, "failed", 10);
        fx.db.with_connection(|c| {
            c.execute("UPDATE turns SET error = 'boom' WHERE id = 't1'", [])
                .unwrap()
        });
        fs::remove_dir_all(&gone.workspace_path).unwrap();
        let agents = fx.list();
        let attention = |name: &str| {
            agents
                .iter()
                .find(|agent| agent.name == name)
                .and_then(|agent| agent.attention.clone())
        };
        assert_eq!(
            attention("Failed").map(|a| (a.code, a.message)),
            Some(("last-turn-failed", "The last reply failed: boom".to_owned()))
        );
        assert_eq!(attention("Gone").unwrap().code, "workspace-missing");
        let value = serde_json::to_value(attention("Gone")).unwrap();
        assert_eq!(value["code"], "workspace-missing");
    }

    #[test]
    fn recreating_a_missing_workspace_makes_the_folder_and_the_instruction_file() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Gone", "claude-code").unwrap();
        fs::remove_dir_all(&agent.workspace_path).unwrap();
        assert_eq!(fx.list()[0].state, AgentState::NeedsAttention);
        let fixed = recreate(&fx.db, &fx.dirs(), &APP_AUTH, &agent.id).unwrap();
        assert_eq!(fixed.state, AgentState::Idle);
        assert_eq!(fixed.attention, None);
        assert_eq!(
            fs::read_to_string(Path::new(&agent.workspace_path).join("CLAUDE.md")).unwrap(),
            workspace::instruction_text("Gone")
        );
    }

    #[test]
    fn recreating_makes_missing_parent_folders_too() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Deep", "claude-code").unwrap();
        let deep = fx.workspaces.path().join("a").join("b").join("deep");
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?2 WHERE id = ?1",
                rusqlite::params![agent.id, deep.to_string_lossy()],
            )
            .unwrap()
        });
        recreate(&fx.db, &fx.dirs(), &APP_AUTH, &agent.id).unwrap();
        assert!(deep.join("CLAUDE.md").is_file());
    }

    #[test]
    fn recreating_never_touches_an_existing_folder_or_instruction_file() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Keep", "claude-code").unwrap();
        let folder = PathBuf::from(&agent.workspace_path);
        fs::write(folder.join("CLAUDE.md"), "my own words").unwrap();
        fs::write(folder.join("notes.txt"), "work").unwrap();
        recreate(&fx.db, &fx.dirs(), &APP_AUTH, &agent.id).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("CLAUDE.md")).unwrap(),
            "my own words"
        );
        assert_eq!(
            fs::read_to_string(folder.join("notes.txt")).unwrap(),
            "work"
        );
        fs::remove_file(folder.join("CLAUDE.md")).unwrap();
        recreate(&fx.db, &fx.dirs(), &APP_AUTH, &agent.id).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("CLAUDE.md")).unwrap(),
            workspace::instruction_text("Keep")
        );
        assert_eq!(
            fs::read_to_string(folder.join("notes.txt")).unwrap(),
            "work"
        );
    }

    #[test]
    fn recreating_for_an_unknown_agent_is_refused() {
        let fx = Fixture::new(&["claude"]);
        let error = recreate(&fx.db, &fx.dirs(), &APP_AUTH, "nope").unwrap_err();
        assert_eq!(error.code, "not-found");
    }

    fn share_path(fx: &Fixture, from: &Agent, to: &Agent) {
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?1 WHERE id = ?2",
                rusqlite::params![from.workspace_path, to.id],
            )
            .unwrap();
        });
    }

    #[test]
    fn deleting_an_agent_whose_folder_another_agent_records_never_trashes_it() {
        let fx = Fixture::new(&["claude"]);
        let first = fx.create("First", "claude-code").unwrap();
        let second = fx.create("Second", "claude-code").unwrap();
        share_path(&fx, &first, &second);
        let outcome = delete_agent_of(&fx, &first, true, &never_trash).unwrap();
        assert!(outcome.folder_kept);
        assert_eq!(
            outcome.reason.as_deref(),
            Some("Another agent uses this folder.")
        );
        assert_eq!(fx.agent_count(), 1);
        assert!(fx
            .workspaces
            .path()
            .join("first")
            .join("CLAUDE.md")
            .is_file());
    }

    #[test]
    fn a_folder_that_contains_or_sits_inside_another_agents_folder_is_kept_too() {
        let fx = Fixture::new(&["claude"]);
        let outer = fx.create("Outer", "claude-code").unwrap();
        let inner = fx.create("Inner", "claude-code").unwrap();
        let nested = fx.workspaces.path().join("outer").join("nested");
        fs::create_dir(&nested).unwrap();
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?1 WHERE id = ?2",
                rusqlite::params![nested.to_str().unwrap(), inner.id],
            )
            .unwrap();
        });
        let outcome = delete_agent_of(&fx, &outer, true, &never_trash).unwrap();
        assert_eq!(
            outcome.reason.as_deref(),
            Some("Another agent uses this folder.")
        );
        assert!(nested.is_dir());
    }

    #[test]
    fn a_folder_nested_in_another_agents_folder_is_kept() {
        let fx = Fixture::new(&["claude"]);
        let outer = fx.create("Outer", "claude-code").unwrap();
        let inner = fx.create("Inner", "claude-code").unwrap();
        let nested = fx.workspaces.path().join("outer").join("nested");
        fs::create_dir(&nested).unwrap();
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?1 WHERE id = ?2",
                rusqlite::params![nested.to_str().unwrap(), inner.id],
            )
            .unwrap();
        });
        let outcome = delete_agent_of(&fx, &inner, true, &never_trash).unwrap();
        assert_eq!(
            outcome.reason.as_deref(),
            Some("Another agent uses this folder.")
        );
        assert!(nested.is_dir());
        assert_eq!(fx.agent_count(), 1);
        assert_eq!(fx.list()[0].id, outer.id);
    }

    #[test]
    fn a_new_agent_skips_a_path_another_agent_records_even_when_the_folder_is_gone() {
        let fx = Fixture::new(&["claude"]);
        let first = fx.create("Test Agent", "claude-code").unwrap();
        fs::remove_dir_all(&first.workspace_path).unwrap();
        let second = fx.create("test-agent", "claude-code").unwrap();
        assert_ne!(second.workspace_path, first.workspace_path);
        assert!(second.workspace_path.ends_with("test-agent-2"));
    }

    #[test]
    fn recreating_refuses_a_path_another_agent_records() {
        let fx = Fixture::new(&["claude"]);
        let first = fx.create("First", "claude-code").unwrap();
        let second = fx.create("Second", "claude-code").unwrap();
        fs::remove_dir_all(&second.workspace_path).unwrap();
        share_path(&fx, &first, &second);
        let error = recreate(&fx.db, &fx.dirs(), &APP_AUTH, &second.id).unwrap_err();
        assert_eq!(error.code, "folder-in-use");
        assert_eq!(error.message, "Another agent uses this folder.");
        assert!(fx
            .workspaces
            .path()
            .join("first")
            .join("CLAUDE.md")
            .is_file());
    }

    #[test]
    fn recreating_reports_a_folder_that_cannot_be_made() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Blocked", "claude-code").unwrap();
        let blocker = fx.workspaces.path().join("file");
        fs::write(&blocker, "x").unwrap();
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?2 WHERE id = ?1",
                rusqlite::params![agent.id, blocker.join("inside").to_string_lossy()],
            )
            .unwrap()
        });
        let error = recreate(&fx.db, &fx.dirs(), &APP_AUTH, &agent.id).unwrap_err();
        assert_eq!(error.code, "folder-failed");
    }

    #[test]
    fn a_new_agent_without_a_choice_follows_the_app_default() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Follower", "claude-code").unwrap();
        assert_eq!(agent.auth_mode, None);
        assert_eq!(agent.effective_auth_mode, AuthMode::Subscription);
        let with_key = AppAuth {
            setting: None,
            key_exists: true,
        };
        let listed = list(&fx.db, &fx.dirs(), &with_key).unwrap();
        assert_eq!(listed[0].auth_mode, None);
        assert_eq!(listed[0].effective_auth_mode, AuthMode::ApiKey);
    }

    #[test]
    fn a_new_agent_can_choose_its_own_sign_in_and_it_survives_a_listing() {
        let fx = Fixture::new(&["claude"]);
        let mut chosen = input("Chooser", "claude-code");
        chosen.auth_mode = Some(AuthMode::ApiKey);
        let agent = create(&fx.db, &fx.root(), chosen, &fx.dirs(), &APP_AUTH).unwrap();
        assert_eq!(agent.auth_mode, Some(AuthMode::ApiKey));
        assert_eq!(agent.effective_auth_mode, AuthMode::ApiKey);
        assert_eq!(fx.list(), vec![agent]);
    }

    #[test]
    fn the_create_input_reads_a_missing_null_or_named_auth_mode() {
        let parse = |json: &str| {
            serde_json::from_str::<CreateAgentInput>(json)
                .unwrap()
                .auth_mode
        };
        let base = r#""name":"a","color":"blue","provider":"claude-code","model":"default""#;
        assert_eq!(parse(&format!("{{{base}}}")), None);
        assert_eq!(parse(&format!("{{{base},\"authMode\":null}}")), None);
        assert_eq!(
            parse(&format!("{{{base},\"authMode\":\"api-key\"}}")),
            Some(AuthMode::ApiKey)
        );
        assert_eq!(
            parse(&format!("{{{base},\"authMode\":\"subscription\"}}")),
            Some(AuthMode::Subscription)
        );
    }

    #[test]
    fn setting_an_agent_override_changes_only_that_agent_and_null_clears_it() {
        let fx = Fixture::new(&["claude"]);
        let one = fx.create("One", "claude-code").unwrap();
        let two = fx.create("Two", "claude-code").unwrap();
        let key_default = AppAuth {
            setting: Some(AuthMode::ApiKey),
            key_exists: true,
        };
        let set = |agent: &Agent, mode| {
            set_auth_mode(&fx.db, &fx.dirs(), &key_default, &agent.id, mode).unwrap()
        };
        let overridden = set(&one, Some(AuthMode::Subscription));
        assert_eq!(overridden.auth_mode, Some(AuthMode::Subscription));
        assert_eq!(overridden.effective_auth_mode, AuthMode::Subscription);
        let listed = list(&fx.db, &fx.dirs(), &key_default).unwrap();
        let other = listed.iter().find(|agent| agent.id == two.id).unwrap();
        assert_eq!(other.auth_mode, None);
        assert_eq!(other.effective_auth_mode, AuthMode::ApiKey);
        let cleared = set(&one, None);
        assert_eq!(cleared.auth_mode, None);
        assert_eq!(cleared.effective_auth_mode, AuthMode::ApiKey);
    }

    #[test]
    fn setting_the_override_of_a_missing_agent_is_not_found() {
        let fx = Fixture::new(&["claude"]);
        let error = set_auth_mode(&fx.db, &fx.dirs(), &APP_AUTH, "nope", None).unwrap_err();
        assert_eq!(error.code, "not-found");
    }

    fn update_input(
        name: Option<&str>,
        color: Option<&str>,
        model: Option<&str>,
    ) -> UpdateAgentInput {
        UpdateAgentInput {
            name: name.map(str::to_string),
            color: color.map(str::to_string),
            model: model.map(str::to_string),
        }
    }

    fn update_agent_of(
        fx: &Fixture,
        agent: &Agent,
        input: UpdateAgentInput,
    ) -> Result<Agent, AgentError> {
        update(&fx.db, &fx.dirs(), &APP_AUTH, &agent.id, input)
    }

    fn count_rows(fx: &Fixture, table: &str) -> i64 {
        fx.db.with_connection(|c| {
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
    }

    fn fill_history(fx: &Fixture, agent: &Agent, status: &str) {
        insert_turn(fx, "turn-1", &agent.id, status, 10);
        fx.db.with_connection(|c| {
            c.execute(
                "INSERT INTO messages (id, agent_id, turn_id, role, content, created_at)
                 VALUES ('message-1', ?1, 'turn-1', 'user', 'hello', 10)",
                [&agent.id],
            )
            .unwrap();
            c.execute(
                "INSERT INTO steps (id, turn_id, position, kind, title, status, created_at)
                 VALUES ('step-1', 'turn-1', 0, 'tool', 'Bash', 'done', 10)",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO queued_messages (id, agent_id, content, position, created_at)
                 VALUES ('queued-1', ?1, 'later', 0, 10)",
                [&agent.id],
            )
            .unwrap();
        });
    }

    #[test]
    fn an_update_changes_name_colour_and_model_and_nothing_else() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let updated = update_agent_of(
            &fx,
            &agent,
            update_input(Some("  Renamed "), Some("red"), Some("opus")),
        )
        .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.color, "red");
        assert_eq!(updated.model, "opus");
        assert_eq!(updated.workspace_path, agent.workspace_path);
        assert_eq!(updated.id, agent.id);
        assert_eq!(updated.created_at, agent.created_at);
        assert_eq!(updated.last_activity_at, agent.last_activity_at);
        assert_eq!(fx.list(), vec![updated]);
    }

    #[test]
    fn an_update_with_one_field_keeps_the_others() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let updated = update_agent_of(&fx, &agent, update_input(None, Some("pink"), None)).unwrap();
        assert_eq!(updated.name, "Test Agent");
        assert_eq!(updated.model, "default");
        assert_eq!(updated.color, "pink");
        let untouched = update_agent_of(&fx, &agent, UpdateAgentInput::default()).unwrap();
        assert_eq!(untouched, updated);
    }

    #[test]
    fn the_update_input_reads_missing_and_named_fields() {
        let none: UpdateAgentInput = serde_json::from_str("{}").unwrap();
        assert_eq!(none.name, None);
        let some: UpdateAgentInput =
            serde_json::from_str(r#"{"name":"A","color":"red","model":"opus"}"#).unwrap();
        assert_eq!(some.name.as_deref(), Some("A"));
        assert_eq!(some.color.as_deref(), Some("red"));
        assert_eq!(some.model.as_deref(), Some("opus"));
    }

    #[test]
    fn an_update_never_renames_or_moves_the_folder() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        update_agent_of(&fx, &agent, update_input(Some("Other Name"), None, None)).unwrap();
        let folder = fx.workspaces.path().join("test-agent");
        assert!(folder.join("CLAUDE.md").is_file());
        assert_eq!(fx.folder_count(), 1);
        assert_eq!(fx.list()[0].workspace_path, folder.to_string_lossy());
    }

    #[test]
    fn an_update_keeps_the_whole_conversation() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        fill_history(&fx, &agent, "completed");
        update_agent_of(
            &fx,
            &agent,
            update_input(Some("Renamed"), Some("teal"), Some("haiku")),
        )
        .unwrap();
        assert_eq!(count_rows(&fx, "turns"), 1);
        assert_eq!(count_rows(&fx, "messages"), 1);
        assert_eq!(count_rows(&fx, "steps"), 1);
        assert_eq!(count_rows(&fx, "queued_messages"), 1);
    }

    #[test]
    fn a_name_used_by_another_agent_is_refused_ignoring_case() {
        let fx = Fixture::new(&["claude"]);
        fx.create("First", "claude-code").unwrap();
        let second = fx.create("Second", "claude-code").unwrap();
        for taken in ["First", "first", "  FIRST "] {
            let error =
                update_agent_of(&fx, &second, update_input(Some(taken), None, None)).unwrap_err();
            assert_eq!(error.code, "name-taken");
        }
        assert_eq!(fx.list().iter().filter(|a| a.name == "Second").count(), 1);
    }

    #[test]
    fn an_agent_may_keep_its_own_name_or_change_its_case() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let same =
            update_agent_of(&fx, &agent, update_input(Some("Test Agent"), None, None)).unwrap();
        assert_eq!(same.name, "Test Agent");
        let cased =
            update_agent_of(&fx, &agent, update_input(Some("TEST AGENT"), None, None)).unwrap();
        assert_eq!(cased.name, "TEST AGENT");
    }

    #[test]
    fn an_update_validates_like_create_and_changes_nothing_on_error() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let code_of_update = |input| update_agent_of(&fx, &agent, input).unwrap_err().code;
        assert_eq!(
            code_of_update(update_input(Some("  "), None, None)),
            "name-empty"
        );
        assert_eq!(
            code_of_update(update_input(Some(&"a".repeat(41)), None, None)),
            "name-too-long"
        );
        assert_eq!(
            code_of_update(update_input(None, Some("purple"), None)),
            "color-invalid"
        );
        assert_eq!(
            code_of_update(update_input(None, None, Some("two words"))),
            "model-invalid"
        );
        assert_eq!(
            code_of_update(update_input(Some("Fine"), Some("purple"), None)),
            "color-invalid"
        );
        assert_eq!(fx.list(), vec![agent]);
    }

    #[test]
    fn updating_a_missing_agent_is_not_found() {
        let fx = Fixture::new(&["claude"]);
        let error = update(
            &fx.db,
            &fx.dirs(),
            &APP_AUTH,
            "nope",
            update_input(Some("A"), None, None),
        )
        .unwrap_err();
        assert_eq!(error.code, "not-found");
    }

    fn never_trash(_: &Path) -> Result<(), String> {
        panic!("the folder must not be moved to the Trash");
    }

    fn delete_agent_of(
        fx: &Fixture,
        agent: &Agent,
        trash: bool,
        trasher: &dyn Fn(&Path) -> Result<(), String>,
    ) -> Result<remove::Deleted, AgentError> {
        remove::delete(&fx.db, &fx.root(), &agent.id, trash, trasher)
    }

    #[test]
    fn deleting_a_working_agent_is_refused_and_changes_nothing() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        fill_history(&fx, &agent, "running");
        for trash in [false, true] {
            let error = delete_agent_of(&fx, &agent, trash, &never_trash).unwrap_err();
            assert_eq!(error.code, "busy");
            assert_eq!(error.message, "Stop the agent before deleting it.");
        }
        assert_eq!(fx.agent_count(), 1);
        assert_eq!(count_rows(&fx, "turns"), 1);
        assert!(fx.workspaces.path().join("test-agent").is_dir());
    }

    #[test]
    fn an_agent_whose_last_turn_ended_can_be_deleted() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        fill_history(&fx, &agent, "stopped");
        delete_agent_of(&fx, &agent, false, &never_trash).unwrap();
        assert_eq!(fx.agent_count(), 0);
    }

    #[test]
    fn deleting_removes_the_row_and_the_whole_conversation() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let other = fx.create("Other", "claude-code").unwrap();
        fill_history(&fx, &agent, "completed");
        delete_agent_of(&fx, &agent, false, &never_trash).unwrap();
        assert_eq!(
            fx.list().iter().map(|a| &a.id).collect::<Vec<_>>(),
            vec![&other.id]
        );
        for table in ["turns", "messages", "steps", "queued_messages"] {
            assert_eq!(count_rows(&fx, table), 0, "{table}");
        }
    }

    #[test]
    fn deleting_without_the_checkbox_keeps_the_folder_and_its_files() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let folder = fx.workspaces.path().join("test-agent");
        fs::write(folder.join("notes.txt"), "keep me").unwrap();
        let outcome = delete_agent_of(&fx, &agent, false, &never_trash).unwrap();
        assert!(outcome.folder_kept);
        assert_eq!(outcome.reason, None);
        assert_eq!(
            fs::read_to_string(folder.join("notes.txt")).unwrap(),
            "keep me"
        );
        assert!(folder.join("CLAUDE.md").is_file());
    }

    #[test]
    fn deleting_with_the_checkbox_hands_the_recorded_folder_to_the_trash() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let folder = fx.workspaces.path().join("test-agent");
        let trashed = std::cell::RefCell::new(Vec::new());
        let outcome = delete_agent_of(&fx, &agent, true, &|path: &Path| {
            trashed.borrow_mut().push(path.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert!(!outcome.folder_kept);
        assert_eq!(outcome.reason, None);
        assert_eq!(*trashed.borrow(), vec![fs::canonicalize(folder).unwrap()]);
        assert_eq!(fx.agent_count(), 0);
    }

    #[test]
    fn a_recorded_folder_outside_the_root_is_kept_but_the_agent_is_deleted() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?1 WHERE id = ?2",
                rusqlite::params![elsewhere.path().to_str().unwrap(), agent.id],
            )
            .unwrap();
        });
        let outcome = delete_agent_of(&fx, &agent, true, &never_trash).unwrap();
        assert!(outcome.folder_kept);
        assert!(outcome.reason.unwrap().contains("outside"));
        assert!(elsewhere.path().is_dir());
        assert_eq!(fx.agent_count(), 0);
    }

    #[test]
    fn a_recorded_symlink_is_kept_and_its_target_is_untouched() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let target = fx.workspaces.path().join("test-agent");
        let link = fx.workspaces.path().join("linked");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET workspace_path = ?1 WHERE id = ?2",
                rusqlite::params![link.to_str().unwrap(), agent.id],
            )
            .unwrap();
        });
        let outcome = delete_agent_of(&fx, &agent, true, &never_trash).unwrap();
        assert!(outcome.folder_kept);
        assert!(target.join("CLAUDE.md").is_file());
        assert!(link.symlink_metadata().unwrap().is_symlink());
    }

    #[test]
    fn a_trash_failure_keeps_the_deleted_row_deleted_and_is_reported() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Test Agent", "claude-code").unwrap();
        let outcome =
            delete_agent_of(&fx, &agent, true, &|_: &Path| Err("no trash".to_string())).unwrap();
        assert!(outcome.folder_kept);
        assert!(outcome.reason.unwrap().contains("no trash"));
        assert_eq!(fx.agent_count(), 0);
        assert!(fx.workspaces.path().join("test-agent").is_dir());
    }

    #[test]
    fn deleting_a_missing_agent_is_not_found() {
        let fx = Fixture::new(&["claude"]);
        let error = remove::delete(&fx.db, &fx.root(), "nope", true, &never_trash).unwrap_err();
        assert_eq!(error.code, "not-found");
    }

    #[test]
    fn the_deleted_outcome_serialises_in_camel_case() {
        let value = serde_json::to_value(remove::Deleted {
            folder_kept: true,
            reason: Some("because".to_string()),
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "folderKept": true, "reason": "because" })
        );
    }
}
