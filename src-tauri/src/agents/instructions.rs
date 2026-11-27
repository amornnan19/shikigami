use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use super::{workspace, AgentError};

pub const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Instructions {
    pub text: String,
    pub modified_at: Option<i64>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub modified_at: Option<i64>,
}

fn path_in(folder: &Path) -> PathBuf {
    folder.join(workspace::instruction_file())
}

fn not_a_file() -> AgentError {
    AgentError::new(
        "not-a-file",
        format!("{} is not a regular file.", workspace::instruction_file()),
    )
}

fn too_large() -> AgentError {
    AgentError::new(
        "too-large",
        format!(
            "{} is larger than {} KiB.",
            workspace::instruction_file(),
            MAX_BYTES / 1024
        ),
    )
}

fn io_failed(error: impl std::fmt::Display) -> AgentError {
    AgentError::new(
        "folder-failed",
        format!("Could not use the instruction file: {error}"),
    )
}

fn micros(metadata: &fs::Metadata) -> Option<i64> {
    let elapsed = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(elapsed.as_micros()).ok()
}

fn open_regular(path: &Path) -> Result<Option<(File, fs::Metadata)>, AgentError> {
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Err(not_a_file()),
        Err(error) => return Err(io_failed(error)),
    };
    let metadata = file.metadata().map_err(io_failed)?;
    if !metadata.is_file() {
        return Err(not_a_file());
    }
    Ok(Some((file, metadata)))
}

pub fn read(folder: &Path) -> Result<Instructions, AgentError> {
    let Some((file, metadata)) = open_regular(&path_in(folder))? else {
        return Ok(Instructions {
            text: String::new(),
            modified_at: None,
        });
    };
    if metadata.len() > MAX_BYTES {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_failed)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large());
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        AgentError::new(
            "not-text",
            format!("{} is not valid UTF-8 text.", workspace::instruction_file()),
        )
    })?;
    Ok(Instructions {
        text,
        modified_at: micros(&metadata),
    })
}

fn current_state(path: &Path) -> Result<Option<(Option<i64>, fs::Permissions)>, AgentError> {
    Ok(open_regular(path)?.map(|(_, metadata)| (micros(&metadata), metadata.permissions())))
}

fn current_modified_at(path: &Path) -> Result<Option<i64>, AgentError> {
    Ok(current_state(path)?.and_then(|(modified, _)| modified))
}

fn changed_on_disk() -> AgentError {
    AgentError::new(
        "changed-on-disk",
        format!(
            "{} changed on disk since you opened it.",
            workspace::instruction_file()
        ),
    )
}

fn write_temp(folder: &Path, text: &str) -> Result<PathBuf, AgentError> {
    let temp = folder.join(format!(
        ".{}.{}.tmp",
        workspace::instruction_file(),
        uuid::Uuid::new_v4().simple()
    ));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        });
    match written {
        Ok(()) => Ok(temp),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(io_failed(error))
        }
    }
}

pub fn write(
    folder: &Path,
    text: &str,
    expected_modified_at: Option<i64>,
    force: bool,
) -> Result<Written, AgentError> {
    write_with(folder, text, expected_modified_at, force, &|| {})
}

fn write_with(
    folder: &Path,
    text: &str,
    expected_modified_at: Option<i64>,
    force: bool,
    before_rename: &dyn Fn(),
) -> Result<Written, AgentError> {
    if text.len() as u64 > MAX_BYTES {
        return Err(too_large());
    }
    let path = path_in(folder);
    let existing = current_state(&path)?;
    let (current, permissions) = match existing {
        Some((modified, permissions)) => (modified, Some(permissions)),
        None => (None, None),
    };
    if permissions
        .as_ref()
        .is_some_and(|permissions| permissions.mode() & 0o222 == 0)
    {
        return Err(AgentError::new(
            "read-only",
            format!("{} is read-only.", workspace::instruction_file()),
        ));
    }
    if !force && current != expected_modified_at {
        return Err(changed_on_disk());
    }
    let temp = write_temp(folder, text)?;
    let finished = permissions
        .map_or(Ok(()), |permissions| {
            fs::set_permissions(&temp, permissions)
        })
        .map_err(io_failed)
        .and_then(|()| {
            before_rename();
            if !force && current_modified_at(&path)? != current {
                return Err(changed_on_disk());
            }
            fs::rename(&temp, &path).map_err(io_failed)
        });
    if let Err(error) = finished {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(Written {
        modified_at: current_modified_at(&path)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{symlink, FileTypeExt, MetadataExt};
    use std::time::{Duration, SystemTime};

    fn folder_with(text: &str) -> tempfile::TempDir {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("CLAUDE.md"), text).unwrap();
        folder
    }

    fn set_mtime(path: &Path, seconds: u64) {
        let file = OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }

    fn code<T>(result: Result<T, AgentError>) -> &'static str {
        result.err().expect("expected an error").code
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_file_reads_as_empty_text() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(
            read(folder.path()).unwrap(),
            Instructions {
                text: String::new(),
                modified_at: None
            }
        );
    }

    #[test]
    fn reading_returns_the_text_and_its_modification_time() {
        let folder = folder_with("hello");
        set_mtime(&folder.path().join("CLAUDE.md"), 1_000);
        let read = read(folder.path()).unwrap();
        assert_eq!(read.text, "hello");
        assert_eq!(read.modified_at, Some(1_000_000_000));
    }

    #[test]
    fn a_file_over_the_limit_is_too_large_to_read_but_the_limit_itself_is_fine() {
        let at_limit = folder_with(&"a".repeat(MAX_BYTES as usize));
        assert_eq!(read(at_limit.path()).unwrap().text.len() as u64, MAX_BYTES);
        let over = folder_with(&"a".repeat(MAX_BYTES as usize + 1));
        assert_eq!(code(read(over.path())), "too-large");
    }

    #[test]
    fn a_file_that_is_not_utf8_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("CLAUDE.md"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(code(read(folder.path())), "not-text");
    }

    #[test]
    fn a_symlink_is_neither_read_nor_written_nor_followed() {
        let folder = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "secret").unwrap();
        symlink(&secret, folder.path().join("CLAUDE.md")).unwrap();
        assert_eq!(code(read(folder.path())), "not-a-file");
        assert_eq!(code(write(folder.path(), "new", None, true)), "not-a-file");
        assert_eq!(fs::read_to_string(&secret).unwrap(), "secret");
        assert!(fs::symlink_metadata(folder.path().join("CLAUDE.md"))
            .unwrap()
            .is_symlink());
    }

    #[test]
    fn a_dangling_symlink_is_refused_too() {
        let folder = tempfile::tempdir().unwrap();
        symlink(
            folder.path().join("nowhere"),
            folder.path().join("CLAUDE.md"),
        )
        .unwrap();
        assert_eq!(code(read(folder.path())), "not-a-file");
        assert_eq!(code(write(folder.path(), "new", None, true)), "not-a-file");
    }

    #[test]
    fn a_directory_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        fs::create_dir(folder.path().join("CLAUDE.md")).unwrap();
        assert_eq!(code(read(folder.path())), "not-a-file");
        assert_eq!(code(write(folder.path(), "new", None, true)), "not-a-file");
        assert!(folder.path().join("CLAUDE.md").is_dir());
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let folder = tempfile::tempdir().unwrap();
        let fifo = folder.path().join("CLAUDE.md");
        let c_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert_eq!(code(read(folder.path())), "not-a-file");
        assert_eq!(code(write(folder.path(), "new", None, true)), "not-a-file");
        assert!(fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo());
    }

    #[test]
    fn writing_with_the_expected_time_replaces_the_text_and_returns_the_new_time() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        set_mtime(&path, 1_000);
        let opened = read(folder.path()).unwrap();
        let written = write(folder.path(), "new text", opened.modified_at, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new text");
        assert_ne!(written.modified_at, opened.modified_at);
        assert_eq!(
            read(folder.path()).unwrap().modified_at,
            written.modified_at
        );
        assert!(write(folder.path(), "again", written.modified_at, false).is_ok());
    }

    #[test]
    fn writing_when_the_file_changed_is_refused_and_writes_nothing() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        set_mtime(&path, 1_000);
        let opened = read(folder.path()).unwrap();
        set_mtime(&path, 2_000);
        assert_eq!(
            code(write(folder.path(), "mine", opened.modified_at, false)),
            "changed-on-disk"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names(folder.path()), vec!["CLAUDE.md"]);
    }

    #[test]
    fn force_overwrites_a_changed_file() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        set_mtime(&path, 1_000);
        let opened = read(folder.path()).unwrap();
        set_mtime(&path, 2_000);
        write(folder.path(), "mine", opened.modified_at, true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
    }

    #[test]
    fn a_file_deleted_or_created_since_opening_counts_as_changed() {
        let folder = folder_with("old");
        let opened = read(folder.path()).unwrap();
        fs::remove_file(folder.path().join("CLAUDE.md")).unwrap();
        assert_eq!(
            code(write(folder.path(), "mine", opened.modified_at, false)),
            "changed-on-disk"
        );
        let empty = tempfile::tempdir().unwrap();
        let opened_empty = read(empty.path()).unwrap();
        fs::write(empty.path().join("CLAUDE.md"), "appeared").unwrap();
        assert_eq!(
            code(write(empty.path(), "mine", opened_empty.modified_at, false)),
            "changed-on-disk"
        );
        assert_eq!(
            fs::read_to_string(empty.path().join("CLAUDE.md")).unwrap(),
            "appeared"
        );
    }

    #[test]
    fn a_missing_file_can_be_created_when_none_was_expected() {
        let folder = tempfile::tempdir().unwrap();
        let written = write(folder.path(), "fresh", None, false).unwrap();
        assert!(written.modified_at.is_some());
        assert_eq!(
            fs::read_to_string(folder.path().join("CLAUDE.md")).unwrap(),
            "fresh"
        );
    }

    #[test]
    fn text_over_the_limit_is_refused_and_the_limit_itself_is_written() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        let opened = read(folder.path()).unwrap();
        let over = "a".repeat(MAX_BYTES as usize + 1);
        assert_eq!(
            code(write(folder.path(), &over, opened.modified_at, true)),
            "too-large"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        let at_limit = "a".repeat(MAX_BYTES as usize);
        write(folder.path(), &at_limit, opened.modified_at, true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().len() as u64, MAX_BYTES);
    }

    #[test]
    fn the_limit_counts_bytes_not_characters() {
        let folder = tempfile::tempdir().unwrap();
        let text = "ก".repeat(MAX_BYTES as usize / 3 + 1);
        assert_eq!(code(write(folder.path(), &text, None, true)), "too-large");
    }

    #[test]
    fn a_write_replaces_the_file_through_a_rename_and_leaves_no_temp_file() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        let before = fs::metadata(&path).unwrap();
        let opened = read(folder.path()).unwrap();
        write(folder.path(), "new", opened.modified_at, false).unwrap();
        assert_eq!(names(folder.path()), vec!["CLAUDE.md"]);
        assert_ne!(before.ino(), fs::metadata(&path).unwrap().ino());
    }

    #[test]
    fn a_change_between_the_check_and_the_rename_is_refused_and_leaves_no_temp_file() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        set_mtime(&path, 1_000);
        let opened = read(folder.path()).unwrap();
        let error = write_with(folder.path(), "mine", opened.modified_at, false, &|| {
            set_mtime(&path, 2_000)
        })
        .unwrap_err();
        assert_eq!(error.code, "changed-on-disk");
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names(folder.path()), vec!["CLAUDE.md"]);
    }

    #[test]
    fn a_forced_write_ignores_a_change_before_the_rename() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        write_with(folder.path(), "mine", None, true, &|| {
            set_mtime(&path, 2_000)
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
    }

    #[test]
    fn a_write_keeps_the_permissions_of_the_file_it_replaces() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let opened = read(folder.path()).unwrap();
        write(folder.path(), "new", opened.modified_at, false).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o640);
    }

    #[test]
    fn a_read_only_file_is_refused_even_when_forced_and_stays_as_it_was() {
        let folder = folder_with("old");
        let path = folder.path().join("CLAUDE.md");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        let opened = read(folder.path()).unwrap();
        for force in [false, true] {
            assert_eq!(
                code(write(folder.path(), "new", opened.modified_at, force)),
                "read-only"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o444);
        assert_eq!(names(folder.path()), vec!["CLAUDE.md"]);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let base = tempfile::tempdir().unwrap();
        let gone = base.path().join("gone");
        assert_eq!(read(&gone).unwrap().modified_at, None);
        assert_eq!(code(write(&gone, "x", None, false)), "folder-failed");
    }
}
