use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const OVERRIDE_VAR: &str = "SHIKIGAMI_PROVIDER_PATH";

const HOME_DIRS: [&str; 4] = [".local/bin", ".bun/bin", ".npm-global/bin", ".volta/bin"];
const SYSTEM_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

pub fn search_dirs() -> Vec<PathBuf> {
    build_search_dirs(
        env::var_os(OVERRIDE_VAR),
        env::var_os("PATH"),
        env::var_os("HOME").map(PathBuf::from),
    )
}

pub fn build_search_dirs(
    override_list: Option<OsString>,
    path: Option<OsString>,
    home: Option<PathBuf>,
) -> Vec<PathBuf> {
    if let Some(list) = override_list {
        return unique(env::split_paths(&list).collect());
    }
    let mut dirs: Vec<PathBuf> = path
        .map(|list| env::split_paths(&list).collect())
        .unwrap_or_default();
    if let Some(home) = &home {
        dirs.push(home.join(HOME_DIRS[0]));
    }
    dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
    if let Some(home) = &home {
        dirs.extend(HOME_DIRS[1..].iter().map(|dir| home.join(dir)));
    }
    unique(dirs)
}

fn unique(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    dirs.into_iter()
        .filter(|dir| !dir.as_os_str().is_empty() && seen.insert(dir.clone()))
        .collect()
}

pub fn find_executable(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn write_file(dir: &Path, name: &str, mode: u32) {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn override_replaces_the_whole_list() {
        let dirs = build_search_dirs(
            Some("/a:/b:/a".into()),
            Some("/usr/bin".into()),
            Some("/home/me".into()),
        );
        assert_eq!(dirs, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn default_list_starts_with_path_and_has_no_duplicates() {
        let dirs = build_search_dirs(
            None,
            Some("/usr/bin:/opt/homebrew/bin".into()),
            Some("/home/me".into()),
        );
        assert_eq!(dirs[0], PathBuf::from("/usr/bin"));
        assert_eq!(dirs[1], PathBuf::from("/opt/homebrew/bin"));
        assert!(dirs.contains(&PathBuf::from("/home/me/.local/bin")));
        assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
        assert!(dirs.contains(&PathBuf::from("/home/me/.volta/bin")));
        let count = dirs
            .iter()
            .filter(|dir| dir.as_path() == Path::new("/opt/homebrew/bin"))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn finds_the_first_executable_and_skips_plain_files() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write_file(first.path(), "tool", 0o644);
        write_file(second.path(), "tool", 0o755);
        let dirs = vec![first.path().to_path_buf(), second.path().to_path_buf()];
        assert_eq!(
            find_executable("tool", &dirs),
            Some(second.path().join("tool"))
        );
        assert_eq!(find_executable("other", &dirs), None);
    }
}
