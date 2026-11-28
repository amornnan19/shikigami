use std::fs;
use std::path::{Path, PathBuf};

use super::GroupError;

pub struct Guard<'a> {
    pub home: Option<&'a Path>,
    pub data_dir: &'a Path,
    pub workspaces: &'a [String],
}

pub fn resolved(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn refused(reason: &str) -> GroupError {
    GroupError::new(
        "folder-refused",
        format!("This folder cannot be used. {reason}"),
    )
}

pub fn check(raw: &str, guard: &Guard) -> Result<PathBuf, GroupError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(GroupError::new(
            "folder-invalid",
            "Choose an existing folder.",
        ));
    }
    let folder = match fs::canonicalize(path) {
        Ok(folder) if folder.is_dir() => folder,
        _ => {
            return Err(GroupError::new(
                "folder-invalid",
                "Choose an existing folder.",
            ))
        }
    };
    if folder.parent().is_none() {
        return Err(refused("It is the disk root."));
    }
    if guard.home.is_some_and(|home| resolved(home) == folder) {
        return Err(refused("It is your home folder."));
    }
    let data = resolved(guard.data_dir);
    if folder == data {
        return Err(refused("It is Shikigami's data folder."));
    }
    if folder.starts_with(&data) {
        return Err(refused("It is inside Shikigami's data folder."));
    }
    if data.starts_with(&folder) {
        return Err(refused("It contains Shikigami's data folder."));
    }
    for workspace in guard.workspaces {
        let workspace = resolved(workspace);
        if folder == workspace {
            return Err(refused("It is an agent's workspace."));
        }
        if folder.starts_with(&workspace) {
            return Err(refused("It is inside an agent's workspace."));
        }
        if workspace.starts_with(&folder) {
            return Err(refused("It contains an agent's workspace."));
        }
    }
    Ok(folder)
}
