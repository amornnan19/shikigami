use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const FOLDER_MAX: usize = 40;
const FALLBACK_FOLDER: &str = "agent";

pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.to_lowercase().chars() {
        let ch = if ch.is_whitespace() || ch == '_' {
            '-'
        } else {
            ch
        };
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
            continue;
        }
        if ch == '-' && slug.ends_with('-') {
            continue;
        }
        slug.push(ch);
    }
    let slug: String = slug.trim_matches('-').chars().take(FOLDER_MAX).collect();
    slug.trim_matches('-').to_string()
}

pub fn folder_name(name: &str) -> String {
    let slug = slug(name);
    if slug.is_empty() {
        FALLBACK_FOLDER.to_string()
    } else {
        slug
    }
}

pub fn instruction_file() -> &'static str {
    "CLAUDE.md"
}

pub fn instruction_text(name: &str) -> String {
    format!("# {name}\n\nYou are {name}, a coding agent working in this folder.\n")
}

pub fn create_folder(root: &Path, name: &str, recorded: &[PathBuf]) -> io::Result<PathBuf> {
    create_folder_named(root, &folder_name(name), recorded)
}

pub fn numbered(root: &Path, base: &str, attempt: u32) -> PathBuf {
    if attempt == 1 {
        root.join(base)
    } else {
        root.join(format!("{base}-{attempt}"))
    }
}

pub fn create_folder_named(root: &Path, base: &str, recorded: &[PathBuf]) -> io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let mut attempt = 1u32;
    loop {
        let path = numbered(root, base, attempt);
        if recorded.contains(&path) {
            attempt += 1;
            continue;
        }
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => attempt += 1,
            Err(error) => return Err(error),
        }
    }
}

pub fn write_instruction_file(folder: &Path, name: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join(instruction_file()))?;
    file.write_all(instruction_text(name).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_slugs() {
        assert_eq!(folder_name("Test Agent"), "test-agent");
        assert_eq!(folder_name("  Hello,   World!  "), "hello-world");
        assert_eq!(folder_name("my_cool__agent"), "my-cool-agent");
        assert_eq!(folder_name("--a--b--"), "a-b");
        assert_eq!(folder_name("Agent 007"), "agent-007");
        assert_eq!(folder_name("../../etc"), "etc");
    }

    #[test]
    fn names_without_usable_characters_become_agent() {
        assert_eq!(folder_name("สวัสดี"), "agent");
        assert_eq!(folder_name("🤖🔥"), "agent");
        assert_eq!(folder_name("!!!"), "agent");
    }

    #[test]
    fn long_names_are_cut_without_a_trailing_dash() {
        let cut = folder_name(&format!("{} tail", "a".repeat(39)));
        assert_eq!(cut, "a".repeat(39));
        assert_eq!(folder_name(&"b".repeat(60)).len(), 40);
    }

    #[test]
    fn colliding_names_get_numbered_folders() {
        let root = tempfile::tempdir().unwrap();
        let first = create_folder(root.path(), "Test Agent", &[]).unwrap();
        let second = create_folder(root.path(), "test agent", &[]).unwrap();
        let third = create_folder(root.path(), "TEST-AGENT", &[]).unwrap();
        assert_eq!(first.file_name().unwrap(), "test-agent");
        assert_eq!(second.file_name().unwrap(), "test-agent-2");
        assert_eq!(third.file_name().unwrap(), "test-agent-3");
    }

    #[test]
    fn a_recorded_path_is_skipped_even_when_its_folder_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let recorded = vec![root.path().join("test-agent")];
        let folder = create_folder(root.path(), "Test Agent", &recorded).unwrap();
        assert_eq!(folder.file_name().unwrap(), "test-agent-2");
        assert!(!recorded[0].exists());
    }

    #[test]
    fn the_root_is_created_when_missing() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("deep").join("agents");
        let folder = create_folder(&root, "a", &[]).unwrap();
        assert!(folder.is_dir());
    }

    #[test]
    fn the_instruction_file_is_claude_md() {
        assert_eq!(instruction_file(), "CLAUDE.md");
    }
}
