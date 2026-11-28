use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::AgentError;
use crate::db::Database;

#[cfg(feature = "e2e")]
use trash as _;

#[cfg(feature = "e2e")]
pub const E2E_TRASH_DIR_ENV: &str = "SHIKIGAMI_E2E_TRASH_DIR";

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deleted {
    pub folder_kept: bool,
    pub reason: Option<String>,
}

impl Deleted {
    fn folder_moved() -> Self {
        Self {
            folder_kept: false,
            reason: None,
        }
    }

    fn folder_left(reason: impl Into<String>) -> Self {
        Self {
            folder_kept: true,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(not(feature = "e2e"))]
pub fn system_trash(path: &Path) -> Result<(), String> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};
    let mut context = trash::TrashContext::default();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context.delete(path).map_err(|error| error.to_string())
}

#[cfg(feature = "e2e")]
pub fn system_trash(path: &Path) -> Result<(), String> {
    let dir = std::env::var_os(E2E_TRASH_DIR_ENV)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{E2E_TRASH_DIR_ENV} is not set in this test build"))?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| "the folder has no name".to_string())?;
    let mut target = dir.join(name);
    let mut attempt = 1u32;
    while target.symlink_metadata().is_ok() {
        attempt += 1;
        target = dir.join(format!("{} {attempt}", name.to_string_lossy()));
    }
    fs::rename(path, target).map_err(|error| error.to_string())
}

fn trash_target(root: &Result<PathBuf, String>, recorded: &str) -> Result<PathBuf, String> {
    let root = root
        .as_ref()
        .map_err(|_| "Shikigami's workspaces folder could not be found.".to_string())?;
    let folder = Path::new(recorded);
    let metadata = match fs::symlink_metadata(folder) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("The folder no longer exists.".to_string());
        }
        Err(error) => return Err(format!("The folder could not be checked: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err("The folder is a link, so it was left in place.".to_string());
    }
    if !metadata.is_dir() {
        return Err("The workspace path is not a folder, so it was left in place.".to_string());
    }
    let (root, folder) = fs::canonicalize(root)
        .and_then(|root| fs::canonicalize(folder).map(|folder| (root, folder)))
        .map_err(|error| format!("The folder could not be checked: {error}"))?;
    if folder == root || !folder.starts_with(&root) {
        return Err(
            "The folder is outside Shikigami's workspaces folder, so it was left in place."
                .to_string(),
        );
    }
    Ok(folder)
}

pub const SHARED_FOLDER_MESSAGE: &str = "Another agent uses this folder.";

fn resolved(path: &str) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

fn overlaps(first: &Path, second: &Path) -> bool {
    first.starts_with(second) || second.starts_with(first)
}

fn delete_row(connection: &Connection, agent_id: &str) -> Result<(String, bool), AgentError> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(AgentError::database)?;
    let recorded: Option<String> = transaction
        .query_row(
            "SELECT workspace_path FROM agents WHERE id = ?1",
            [agent_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(AgentError::database)?;
    let Some(recorded) = recorded else {
        return Err(AgentError::new("not-found", "This agent no longer exists."));
    };
    let running: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE agent_id = ?1 AND status = 'running')",
            [agent_id],
            |row| row.get(0),
        )
        .map_err(AgentError::database)?;
    if running {
        return Err(AgentError::new(
            "busy",
            "Stop the agent before deleting it.",
        ));
    }
    let others: Vec<String> = transaction
        .prepare("SELECT workspace_path FROM agents WHERE id <> ?1")
        .and_then(|mut statement| {
            statement
                .query_map([agent_id], |row| row.get(0))
                .and_then(|rows| rows.collect())
        })
        .map_err(AgentError::database)?;
    let target = resolved(&recorded);
    let shared = others
        .iter()
        .any(|other| overlaps(&target, &resolved(other)));
    crate::groups::store::reassign_leads(&transaction, agent_id).map_err(AgentError::database)?;
    transaction
        .execute("DELETE FROM agents WHERE id = ?1", [agent_id])
        .map_err(AgentError::database)?;
    transaction.commit().map_err(AgentError::database)?;
    Ok((recorded, shared))
}

pub fn delete(
    db: &Database,
    root: &Result<PathBuf, String>,
    agent_id: &str,
    move_folder_to_trash: bool,
    trasher: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Deleted, AgentError> {
    let (recorded, shared) = db.with_connection(|connection| delete_row(connection, agent_id))?;
    if !move_folder_to_trash {
        return Ok(Deleted {
            folder_kept: true,
            reason: None,
        });
    }
    if shared {
        return Ok(Deleted::folder_left(SHARED_FOLDER_MESSAGE));
    }
    Ok(match trash_target(root, &recorded) {
        Err(reason) => Deleted::folder_left(reason),
        Ok(folder) => match trasher(&folder) {
            Ok(()) => Deleted::folder_moved(),
            Err(error) => Deleted::folder_left(format!(
                "The folder could not be moved to the Trash: {error}"
            )),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn root_of(dir: &tempfile::TempDir) -> Result<PathBuf, String> {
        Ok(dir.path().to_path_buf())
    }

    fn reason<T: std::fmt::Debug>(result: Result<T, String>) -> String {
        result.expect_err("expected the folder to be refused")
    }

    #[test]
    fn a_folder_inside_the_root_may_be_trashed() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("agent");
        fs::create_dir(&folder).unwrap();
        let target = trash_target(&root_of(&root), folder.to_str().unwrap()).unwrap();
        assert_eq!(target, fs::canonicalize(&folder).unwrap());
    }

    #[test]
    fn the_root_itself_is_never_trashed() {
        let root = tempfile::tempdir().unwrap();
        let recorded = root.path().to_str().unwrap();
        assert!(reason(trash_target(&root_of(&root), recorded)).contains("outside"));
        let dotted = format!("{recorded}/agent/..");
        fs::create_dir(root.path().join("agent")).unwrap();
        assert!(reason(trash_target(&root_of(&root), &dotted)).contains("outside"));
    }

    #[test]
    fn a_folder_outside_the_root_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let recorded = elsewhere.path().to_str().unwrap();
        assert!(reason(trash_target(&root_of(&root), recorded)).contains("outside"));
    }

    #[test]
    fn a_sibling_with_the_same_prefix_is_outside_the_root() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("agents");
        let sibling = base.path().join("agents-other");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&sibling).unwrap();
        assert!(reason(trash_target(&Ok(root), sibling.to_str().unwrap())).contains("outside"));
    }

    #[test]
    fn a_symlink_to_a_folder_inside_the_root_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(reason(trash_target(&root_of(&root), link.to_str().unwrap())).contains("link"));
    }

    #[test]
    fn a_symlink_to_a_folder_outside_the_root_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let link = root.path().join("link");
        symlink(elsewhere.path(), &link).unwrap();
        assert!(reason(trash_target(&root_of(&root), link.to_str().unwrap())).contains("link"));
    }

    #[test]
    fn a_path_through_a_linked_parent_is_outside_the_root() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::create_dir(elsewhere.path().join("victim")).unwrap();
        symlink(elsewhere.path(), root.path().join("hop")).unwrap();
        let recorded = root.path().join("hop").join("victim");
        assert!(
            reason(trash_target(&root_of(&root), recorded.to_str().unwrap())).contains("outside")
        );
    }

    #[test]
    fn a_missing_path_and_a_plain_file_are_kept() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("gone");
        assert!(
            reason(trash_target(&root_of(&root), missing.to_str().unwrap()))
                .contains("no longer exists")
        );
        let file = root.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(
            reason(trash_target(&root_of(&root), file.to_str().unwrap())).contains("not a folder")
        );
    }

    #[test]
    fn an_unresolved_root_keeps_the_folder() {
        let folder = tempfile::tempdir().unwrap();
        let root = Err("could not find your home folder".to_string());
        assert!(reason(trash_target(&root, folder.path().to_str().unwrap()))
            .contains("workspaces folder"));
    }
}
