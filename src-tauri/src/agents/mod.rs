mod store;
mod validate;
mod workspace;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::db::Database;
use crate::providers;

pub const WORKSPACES_DIR_ENV: &str = "SHIKIGAMI_WORKSPACES_DIR";

pub struct WorkspacesRoot(pub Result<PathBuf, String>);

pub fn resolve_workspaces_root(home: Option<PathBuf>) -> Result<PathBuf, String> {
    match std::env::var_os(WORKSPACES_DIR_ENV) {
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentInput {
    pub name: String,
    pub color: String,
    pub provider: String,
    pub model: String,
}

struct Validated {
    name: String,
    color: String,
    provider: String,
    model: String,
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
    })
}

fn persist(db: &Database, root: &Path, valid: Validated) -> Result<Agent, AgentError> {
    let folder = workspace::create_folder(root, &valid.name).map_err(AgentError::folder)?;
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
    };
    let saved = workspace::write_instruction_file(&folder, &agent.name)
        .map_err(AgentError::folder)
        .and_then(|()| db.with_connection(|connection| store::insert(connection, &agent)));
    if let Err(error) = saved {
        let _ = fs::remove_dir_all(&folder);
        return Err(error);
    }
    Ok(agent)
}

fn create(
    db: &Database,
    root: &Result<PathBuf, String>,
    input: CreateAgentInput,
    dirs: &[PathBuf],
) -> Result<Agent, AgentError> {
    let valid = validate(db, input, dirs)?;
    let root = root
        .as_ref()
        .map_err(|message| AgentError::new("folder-failed", message.clone()))?;
    persist(db, root, valid)
}

fn list(db: &Database, dirs: &[PathBuf]) -> Result<Vec<Agent>, AgentError> {
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
            }
        })
        .collect())
}

fn recreate(db: &Database, dirs: &[PathBuf], agent_id: &str) -> Result<Agent, AgentError> {
    let (name, path) = db
        .with_connection(|connection| store::workspace_of(connection, agent_id))
        .map_err(AgentError::database)?
        .ok_or_else(|| AgentError::new("not-found", "This agent no longer exists."))?;
    let folder = PathBuf::from(path);
    fs::create_dir_all(&folder).map_err(AgentError::folder)?;
    match workspace::write_instruction_file(&folder, &name) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
            return Err(AgentError::folder(error));
        }
        _ => {}
    }
    list(db, dirs)?
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| AgentError::new("not-found", "This agent no longer exists."))
}

fn unavailable() -> AgentError {
    AgentError::new("unavailable", "The app database is not available.")
}

#[tauri::command]
pub async fn list_agents(app: AppHandle) -> Result<Vec<Agent>, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        list(&db, &providers::search_dirs())
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn create_agent(app: AppHandle, input: CreateAgentInput) -> Result<Agent, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let root = app.try_state::<WorkspacesRoot>().ok_or_else(unavailable)?;
        create(&db, &root.0, input, &providers::search_dirs())
    })
    .await
    .map_err(AgentError::database)?
}

#[tauri::command]
pub async fn recreate_workspace(app: AppHandle, agent_id: String) -> Result<Agent, AgentError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        recreate(&db, &providers::search_dirs(), &agent_id)
    })
    .await
    .map_err(AgentError::database)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
            create(&self.db, &self.root(), input(name, provider), &self.dirs())
        }

        fn list(&self) -> Vec<Agent> {
            list(&self.db, &self.dirs()).unwrap()
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
            error_code(fx.create("A", "unknown")),
            "provider-unavailable"
        );
        let mut bad_color = input("A", "claude-code");
        bad_color.color = "purple".to_string();
        assert_eq!(
            error_code(create(&fx.db, &fx.root(), bad_color, &fx.dirs())),
            "color-invalid"
        );
        let mut bad_model = input("A", "claude-code");
        bad_model.model = "two words".to_string();
        assert_eq!(
            error_code(create(&fx.db, &fx.root(), bad_model, &fx.dirs())),
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
        };
        let error = persist(&fx.db, fx.workspaces.path(), valid).unwrap_err();
        assert_eq!(error.code, "name-taken");
        assert_eq!(fx.agent_count(), 1);
        let folders: Vec<_> = fs::read_dir(fx.workspaces.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(folders, vec![std::ffi::OsString::from("taken")]);
    }

    #[test]
    fn a_folder_that_cannot_be_made_leaves_no_agent() {
        let fx = Fixture::new(&["claude"]);
        let blocker = fx.workspaces.path().join("blocker");
        fs::write(&blocker, "a file, not a folder").unwrap();
        let root = Ok(blocker);
        let error = create(&fx.db, &root, input("A", "claude-code"), &fx.dirs()).unwrap_err();
        assert_eq!(error.code, "folder-failed");
        assert_eq!(fx.agent_count(), 0);
    }

    #[test]
    fn an_unresolved_root_is_an_error_not_a_panic() {
        let fx = Fixture::new(&["claude"]);
        let root = Err("could not find your home folder".to_string());
        let error = create(&fx.db, &root, input("A", "claude-code"), &fx.dirs()).unwrap_err();
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

        let without_cli = list(&fx.db, &[]).unwrap();
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
        create(&db, &root, input("Persisted", "claude-code"), &dirs).unwrap();
        drop(db);
        let db = Database::open(data.path()).unwrap();
        let agents = list(&db, &dirs).unwrap();
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
        std::env::remove_var(WORKSPACES_DIR_ENV);
        assert_eq!(
            resolve_workspaces_root(Some(PathBuf::from("/Users/me"))).unwrap(),
            PathBuf::from("/Users/me/Shikigami/agents")
        );
        assert!(resolve_workspaces_root(None).is_err());
        std::env::set_var(WORKSPACES_DIR_ENV, "relative/dir");
        assert!(resolve_workspaces_root(Some(PathBuf::from("/Users/me"))).is_err());
        std::env::set_var(WORKSPACES_DIR_ENV, "/tmp/elsewhere");
        assert_eq!(
            resolve_workspaces_root(None).unwrap(),
            PathBuf::from("/tmp/elsewhere")
        );
        std::env::remove_var(WORKSPACES_DIR_ENV);
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
        let fixed = recreate(&fx.db, &fx.dirs(), &agent.id).unwrap();
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
        recreate(&fx.db, &fx.dirs(), &agent.id).unwrap();
        assert!(deep.join("CLAUDE.md").is_file());
    }

    #[test]
    fn recreating_never_touches_an_existing_folder_or_instruction_file() {
        let fx = Fixture::new(&["claude"]);
        let agent = fx.create("Keep", "claude-code").unwrap();
        let folder = PathBuf::from(&agent.workspace_path);
        fs::write(folder.join("CLAUDE.md"), "my own words").unwrap();
        fs::write(folder.join("notes.txt"), "work").unwrap();
        recreate(&fx.db, &fx.dirs(), &agent.id).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("CLAUDE.md")).unwrap(),
            "my own words"
        );
        assert_eq!(
            fs::read_to_string(folder.join("notes.txt")).unwrap(),
            "work"
        );
        fs::remove_file(folder.join("CLAUDE.md")).unwrap();
        recreate(&fx.db, &fx.dirs(), &agent.id).unwrap();
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
        let error = recreate(&fx.db, &fx.dirs(), "nope").unwrap_err();
        assert_eq!(error.code, "not-found");
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
        let error = recreate(&fx.db, &fx.dirs(), &agent.id).unwrap_err();
        assert_eq!(error.code, "folder-failed");
    }
}
