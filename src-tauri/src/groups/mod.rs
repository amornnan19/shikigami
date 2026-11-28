mod folder;
pub(crate) mod mentions;
pub(crate) mod store;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::agents::{now_millis, validate, workspace, AgentError, Attention};
use crate::db::Database;
use crate::DataDir;

pub const GROUPS_DIR_ENV: &str = "SHIKIGAMI_GROUPS_DIR";

#[cfg(feature = "e2e")]
pub const E2E_PICK_FOLDER_ENV: &str = "SHIKIGAMI_E2E_PICK_FOLDER";

const FALLBACK_FOLDER: &str = "group";
const MEMBERS_MIN: usize = 2;

pub struct GroupsConfig {
    pub root: Result<PathBuf, String>,
    pub home: Option<PathBuf>,
}

pub fn resolve_groups_root(
    home: Option<&Path>,
    override_dir: Option<OsString>,
) -> Result<PathBuf, String> {
    match override_dir {
        Some(dir) if !dir.is_empty() => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                Ok(path)
            } else {
                Err(format!(
                    "{GROUPS_DIR_ENV} must be an absolute path, but it is {}",
                    path.display()
                ))
            }
        }
        _ => home
            .filter(|home| home.is_absolute())
            .map(|home| home.join("Shikigami").join("groups"))
            .ok_or_else(|| "could not find your home folder".to_string()),
    }
}

#[derive(Debug, Serialize)]
pub struct GroupError {
    pub code: &'static str,
    pub message: String,
}

impl GroupError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn name_taken() -> Self {
        Self::new("name-taken", "A group with this name already exists.")
    }

    pub fn database(error: impl std::fmt::Display) -> Self {
        Self::new(
            "database-failed",
            format!("Could not save the group: {error}"),
        )
    }

    fn not_found() -> Self {
        Self::new("not-found", "This group no longer exists.")
    }

    fn folder(error: impl std::fmt::Display) -> Self {
        Self::new(
            "folder-failed",
            format!("Could not make the group's folder: {error}"),
        )
    }
}

impl From<AgentError> for GroupError {
    fn from(error: AgentError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

impl From<rusqlite::Error> for GroupError {
    fn from(error: rusqlite::Error) -> Self {
        Self::database(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub agent_id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub folder_path: String,
    pub members: Vec<Member>,
    pub lead_agent_id: Option<String>,
    pub created_at: i64,
    pub last_activity_at: i64,
    pub attention: Option<Attention>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub group: Group,
    pub shared_folder_with: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroupInput {
    pub name: String,
    #[serde(default)]
    pub folder_path: Option<String>,
    pub member_ids: Vec<String>,
    pub lead_agent_id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGroupInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub member_ids: Option<Vec<String>>,
    #[serde(default)]
    pub lead_agent_id: Option<String>,
}

pub(crate) fn derive_attention(folder_exists: bool, member_count: usize) -> Option<Attention> {
    if !folder_exists {
        return Some(Attention {
            code: "folder-missing",
            message: "The group folder is missing.".to_owned(),
        });
    }
    if member_count < MEMBERS_MIN {
        return Some(Attention {
            code: "too-few-members",
            message: "Add another agent to use this group.".to_owned(),
        });
    }
    None
}

fn list_with(connection: &rusqlite::Connection) -> Result<Vec<Group>, GroupError> {
    Ok(store::list(connection)?
        .into_iter()
        .map(|(row, members)| Group {
            attention: derive_attention(Path::new(&row.folder_path).is_dir(), members.len()),
            id: row.id,
            name: row.name,
            folder_path: row.folder_path,
            members,
            lead_agent_id: row.lead_agent_id,
            created_at: row.created_at,
            last_activity_at: row.last_activity_at,
        })
        .collect())
}

fn find_with(connection: &rusqlite::Connection, group_id: &str) -> Result<Group, GroupError> {
    list_with(connection)?
        .into_iter()
        .find(|group| group.id == group_id)
        .ok_or_else(GroupError::not_found)
}

fn list(db: &Database) -> Result<Vec<Group>, GroupError> {
    db.with_connection(list_with)
}

fn distinct(ids: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

fn check_members(
    connection: &rusqlite::Connection,
    ids: Vec<String>,
) -> Result<Vec<String>, GroupError> {
    let members = distinct(ids);
    if members.len() < MEMBERS_MIN {
        return Err(GroupError::new(
            "members-too-few",
            "Choose at least two agents.",
        ));
    }
    for id in &members {
        if !store::agent_exists(connection, id)? {
            return Err(GroupError::new(
                "member-not-found",
                "One of the chosen agents no longer exists.",
            ));
        }
    }
    Ok(members)
}

fn check_lead(members: &[String], lead: &str) -> Result<(), GroupError> {
    if members.iter().any(|member| member == lead) {
        Ok(())
    } else {
        Err(GroupError::new(
            "lead-not-member",
            "The lead must be one of the members.",
        ))
    }
}

fn folder_base(name: &str) -> String {
    let slug = workspace::slug(name);
    if slug.is_empty() {
        FALLBACK_FOLDER.to_string()
    } else {
        slug
    }
}

fn recorded_folders(workspaces: &[String], groups: &[(String, String)]) -> Vec<PathBuf> {
    workspaces
        .iter()
        .cloned()
        .chain(groups.iter().map(|(_, path)| path.clone()))
        .map(PathBuf::from)
        .collect()
}

fn guard<'a>(
    config: &'a GroupsConfig,
    data_dir: &'a Path,
    workspaces: &'a [String],
) -> folder::Guard<'a> {
    folder::Guard {
        home: config.home.as_deref(),
        data_dir,
        workspaces,
    }
}

fn default_folder(
    config: &GroupsConfig,
    data_dir: &Path,
    name: &str,
    workspaces: &[String],
    groups: &[(String, String)],
) -> Result<PathBuf, GroupError> {
    let root = config
        .root
        .as_ref()
        .map_err(|message| GroupError::new("folder-failed", message.clone()))?;
    let recorded = recorded_folders(workspaces, groups);
    let created = workspace::create_folder_named(root, &folder_base(name), &recorded)
        .map_err(GroupError::folder)?;
    match folder::check(
        &created.to_string_lossy(),
        &guard(config, data_dir, workspaces),
    ) {
        Ok(checked) => Ok(checked),
        Err(error) => {
            let _ = fs::remove_dir(&created);
            Err(error)
        }
    }
}

fn suggest(db: &Database, config: &GroupsConfig, name: &str) -> Result<String, GroupError> {
    let root = config
        .root
        .as_ref()
        .map_err(|message| GroupError::new("folder-failed", message.clone()))?;
    let recorded = db.with_connection(|connection| {
        let workspaces = store::agent_workspaces(connection)?;
        let groups = store::group_folders(connection, None)?;
        Ok::<_, GroupError>(recorded_folders(&workspaces, &groups))
    })?;
    let base = folder_base(name);
    let mut attempt = 1u32;
    loop {
        let path = workspace::numbered(root, &base, attempt);
        if !recorded.contains(&path) && path.symlink_metadata().is_err() {
            return Ok(path.to_string_lossy().into_owned());
        }
        attempt += 1;
    }
}

fn create(
    db: &Database,
    config: &GroupsConfig,
    data_dir: &Path,
    input: CreateGroupInput,
) -> Result<Created, GroupError> {
    let name = validate::name(&input.name)?;
    db.with_connection(|connection| {
        if store::name_taken_by_other(connection, &name, None)? {
            return Err(GroupError::name_taken());
        }
        let members = check_members(connection, input.member_ids)?;
        check_lead(&members, &input.lead_agent_id)?;
        let workspaces = store::agent_workspaces(connection)?;
        let groups = store::group_folders(connection, None)?;
        let (folder, made_here) = match input.folder_path.as_deref() {
            Some(raw) => (
                folder::check(raw, &guard(config, data_dir, &workspaces))?,
                false,
            ),
            None => (
                default_folder(config, data_dir, &name, &workspaces, &groups)?,
                true,
            ),
        };
        let shared_folder_with = groups
            .into_iter()
            .filter(|(_, path)| folder::resolved(path) == folder)
            .map(|(name, _)| name)
            .collect();
        let now = now_millis();
        let row = store::GroupRow {
            id: uuid::Uuid::now_v7().to_string(),
            name,
            folder_path: folder.to_string_lossy().into_owned(),
            lead_agent_id: Some(input.lead_agent_id),
            created_at: now,
            last_activity_at: now,
        };
        let saved = connection
            .unchecked_transaction()
            .map_err(GroupError::database)
            .and_then(|transaction| {
                store::insert(&transaction, &row, &members)?;
                transaction.commit().map_err(GroupError::database)
            });
        if let Err(error) = saved {
            if made_here {
                let _ = fs::remove_dir(&folder);
            }
            return Err(error);
        }
        Ok(Created {
            group: find_with(connection, &row.id)?,
            shared_folder_with,
        })
    })
}

fn update(db: &Database, group_id: &str, input: UpdateGroupInput) -> Result<Group, GroupError> {
    let name = input.name.as_deref().map(validate::name).transpose()?;
    db.with_connection(|connection| {
        let transaction = connection.unchecked_transaction()?;
        let row = store::find_row(&transaction, group_id)?.ok_or_else(GroupError::not_found)?;
        if let Some(name) = &name {
            if store::name_taken_by_other(&transaction, name, Some(group_id))? {
                return Err(GroupError::name_taken());
            }
        }
        let members = match input.member_ids {
            Some(ids) => Some(check_members(&transaction, ids)?),
            None => None,
        };
        let current_members = match &members {
            Some(members) => members.clone(),
            None => store::member_ids(&transaction, group_id)?,
        };
        let lead = match input.lead_agent_id {
            Some(lead) => Some(lead),
            None if members.is_some() => row.lead_agent_id.clone(),
            None => None,
        };
        if let Some(lead) = &lead {
            check_lead(&current_members, lead)?;
        } else if members.is_some() {
            return Err(GroupError::new(
                "lead-not-member",
                "The lead must be one of the members.",
            ));
        }
        if let Some(name) = &name {
            store::rename(&transaction, group_id, name)?;
        }
        if let Some(members) = &members {
            store::replace_members(&transaction, group_id, members, now_millis())?;
        }
        if let Some(lead) = &lead {
            store::set_lead(&transaction, group_id, Some(lead))?;
        }
        store::readdress_queue(&transaction, group_id)?;
        transaction.commit()?;
        find_with(connection, group_id)
    })
}

fn delete(db: &Database, group_id: &str) -> Result<(), GroupError> {
    let removed = db.with_connection(|connection| {
        let transaction = connection.unchecked_transaction()?;
        if store::has_running_turn(&transaction, group_id)? {
            return Err(GroupError::new(
                "busy",
                "Stop the group's work before deleting it.",
            ));
        }
        let removed = store::delete(&transaction, group_id)?;
        transaction.commit()?;
        Ok::<_, GroupError>(removed)
    })?;
    if removed == 0 {
        return Err(GroupError::not_found());
    }
    Ok(())
}

fn unavailable() -> GroupError {
    GroupError::new("unavailable", "The app database is not available.")
}

#[cfg(not(feature = "e2e"))]
fn pick_folder(app: &AppHandle) -> Result<Option<String>, GroupError> {
    use tauri_plugin_dialog::DialogExt;
    match app.dialog().file().blocking_pick_folder() {
        None => Ok(None),
        Some(chosen) => chosen
            .into_path()
            .map(|path| Some(path.to_string_lossy().into_owned()))
            .map_err(|error| GroupError::new("folder-invalid", error.to_string())),
    }
}

#[cfg(feature = "e2e")]
fn pick_folder(_app: &AppHandle) -> Result<Option<String>, GroupError> {
    Ok(std::env::var(E2E_PICK_FOLDER_ENV)
        .ok()
        .filter(|path| !path.is_empty()))
}

#[tauri::command]
pub async fn list_groups(app: AppHandle) -> Result<Vec<Group>, GroupError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        list(&db)
    })
    .await
    .map_err(GroupError::database)?
}

#[tauri::command]
pub async fn create_group(app: AppHandle, input: CreateGroupInput) -> Result<Created, GroupError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let config = app.try_state::<GroupsConfig>().ok_or_else(unavailable)?;
        let data_dir = app.try_state::<DataDir>().ok_or_else(unavailable)?;
        create(&db, &config, &data_dir.0, input)
    })
    .await
    .map_err(GroupError::database)?
}

#[tauri::command]
pub async fn update_group(
    app: AppHandle,
    group_id: String,
    input: UpdateGroupInput,
) -> Result<Group, GroupError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        update(&db, &group_id, input)
    })
    .await
    .map_err(GroupError::database)?
}

#[tauri::command]
pub async fn delete_group(app: AppHandle, group_id: String) -> Result<(), GroupError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        delete(&db, &group_id)
    })
    .await
    .map_err(GroupError::database)?
}

#[tauri::command]
pub async fn pick_group_folder(app: AppHandle) -> Result<Option<String>, GroupError> {
    tauri::async_runtime::spawn_blocking(move || pick_folder(&app))
        .await
        .map_err(GroupError::database)?
}

#[tauri::command]
pub async fn suggest_group_folder(app: AppHandle, name: String) -> Result<String, GroupError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        let config = app.try_state::<GroupsConfig>().ok_or_else(unavailable)?;
        suggest(&db, &config, &name)
    })
    .await
    .map_err(GroupError::database)?
}

#[cfg(test)]
mod tests;
