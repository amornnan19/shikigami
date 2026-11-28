use std::ffi::CStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::GroupError;

pub struct Guard<'a> {
    pub home: Option<&'a Path>,
    pub data_dir: &'a Path,
    pub workspaces: &'a [String],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Relation {
    Equal,
    Inside,
    Contains,
}

pub fn resolved(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(mut base) = fs::canonicalize(&current) {
            base.extend(missing.iter().rev());
            return base;
        }
        match (current.file_name(), current.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name.to_owned());
                current = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}

fn chain(path: &Path) -> Vec<(u64, u64)> {
    path.ancestors().filter_map(identity).collect()
}

fn relation(folder: &Path, protected: &Path) -> Option<Relation> {
    let protected = resolved(protected);
    if folder == protected {
        return Some(Relation::Equal);
    }
    if folder.starts_with(&protected) {
        return Some(Relation::Inside);
    }
    if protected.starts_with(folder) {
        return Some(Relation::Contains);
    }
    let (Some(folder_id), Some(protected_id)) = (identity(folder), identity(&protected)) else {
        return None;
    };
    if folder_id == protected_id {
        return Some(Relation::Equal);
    }
    if chain(folder).contains(&protected_id) {
        return Some(Relation::Inside);
    }
    if chain(&protected).contains(&folder_id) {
        return Some(Relation::Contains);
    }
    None
}

fn is_mount_root(folder: &Path) -> bool {
    let differs = folder
        .parent()
        .and_then(identity)
        .zip(identity(folder))
        .is_some_and(|(parent, own)| parent.0 != own.0);
    differs || mounted_at(folder)
}

fn mounted_at(folder: &Path) -> bool {
    let Ok(path) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stats) } != 0 {
        return false;
    }
    let mount = unsafe { CStr::from_ptr(stats.f_mntonname.as_ptr()) };
    mount.to_bytes() == folder.as_os_str().as_bytes()
}

fn refused(reason: &str) -> GroupError {
    GroupError::new(
        "folder-refused",
        format!("This folder cannot be used. {reason}"),
    )
}

fn invalid() -> GroupError {
    GroupError::new("folder-invalid", "Choose an existing folder.")
}

pub fn check(raw: &str, guard: &Guard) -> Result<PathBuf, GroupError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(invalid());
    }
    let folder = match fs::canonicalize(path) {
        Ok(folder) if folder.is_dir() => folder,
        _ => return Err(invalid()),
    };
    if folder.parent().is_none() {
        return Err(refused("It is the disk root."));
    }
    if is_mount_root(&folder) {
        return Err(refused("It is the root of a disk."));
    }
    if guard
        .home
        .is_some_and(|home| relation(&folder, home) == Some(Relation::Equal))
    {
        return Err(refused("It is your home folder."));
    }
    if crate::sandbox::has_unsafe_characters(&folder) {
        return Err(refused(
            "Its name contains characters the sandbox cannot use safely.",
        ));
    }
    for location in crate::sandbox::fixed_protected(guard.home) {
        if matches!(
            relation(&folder, &location),
            Some(Relation::Equal | Relation::Inside)
        ) {
            return Err(refused(crate::sandbox::PROTECTED_FOLDER_MESSAGE));
        }
    }
    match relation(&folder, guard.data_dir) {
        Some(Relation::Equal) => return Err(refused("It is Shikigami's data folder.")),
        Some(Relation::Inside) => return Err(refused("It is inside Shikigami's data folder.")),
        Some(Relation::Contains) => {
            return Err(refused("It contains Shikigami's data folder."));
        }
        None => {}
    }
    for workspace in guard.workspaces {
        match relation(&folder, Path::new(workspace)) {
            Some(Relation::Equal) => return Err(refused("It is an agent's workspace.")),
            Some(Relation::Inside) => return Err(refused("It is inside an agent's workspace.")),
            Some(Relation::Contains) => {
                return Err(refused("It contains an agent's workspace."));
            }
            None => {}
        }
    }
    Ok(folder)
}
