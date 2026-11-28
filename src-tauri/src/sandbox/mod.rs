use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const UNAVAILABLE_MESSAGE: &str = "The sandbox could not start, so this agent did not run.";
pub const UNSAFE_FOLDER_MESSAGE: &str =
    "This folder's name contains characters the sandbox cannot use safely.";
pub const PROTECTED_FOLDER_MESSAGE: &str = "It is inside a protected location.";

const UNAVAILABLE_MARKERS: [&str; 6] = [
    "sandbox required but unavailable",
    "refusing to start without a working sandbox",
    "sandbox is required but failed to initialize",
    "sandbox is required (sandbox.failifunavailable)",
    "did not load, so nothing in them is in effect",
    "these settings entries were skipped",
];

const PROTECTED_IN_HOME: [&str; 68] = [
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config/gcloud",
    ".config/gh",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".git-credentials",
    ".claude",
    ".claude.json",
    "Library/Keychains",
    "Library/Mail",
    "Library/Messages",
    "Library/Safari",
    "Library/Application Support/Google",
    "Library/Application Support/Firefox",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Cookies",
    ".zsh_history",
    ".bash_history",
    ".zsh_sessions",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".bashrc",
    ".bash_profile",
    ".profile",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".gem/credentials",
    ".m2/settings.xml",
    ".gradle/gradle.properties",
    ".yarnrc.yml",
    ".config/pip",
    ".composer/auth.json",
    ".oci",
    ".config/doctl",
    ".terraform.d",
    ".vault-token",
    ".s3cfg",
    ".boto",
    ".pgpass",
    ".my.cnf",
    ".config/git/credentials",
    ".codex",
    ".config/github-copilot",
    "Library/Containers/com.apple.Safari",
    "Library/Application Support/Chromium",
    "Library/Application Support/Vivaldi",
    "Library/Application Support/com.operasoftware.Opera",
    "Library/Group Containers",
    ".bash_login",
    ".bash_aliases",
    ".zlogout",
    ".local/share/fish",
    ".config/fish",
    ".python_history",
    ".psql_history",
    ".node_repl_history",
    ".lesshst",
    ".viminfo",
];

const PROTECTED_ABSOLUTE: [&str; 1] = ["/Volumes"];

#[derive(Debug, Clone)]
pub struct Scope {
    pub home: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub temp: PathBuf,
}

impl Scope {
    pub fn current(data_dir: PathBuf) -> Self {
        Self {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute()),
            data_dir,
            temp: std::env::temp_dir(),
        }
    }
}

pub fn real_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn is_unavailable(text: &str) -> bool {
    let lowered = text.to_lowercase();
    UNAVAILABLE_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

pub fn fixed_protected(home: Option<&Path>) -> Vec<PathBuf> {
    let in_home = home
        .into_iter()
        .flat_map(|home| PROTECTED_IN_HOME.iter().map(|relative| home.join(relative)));
    in_home
        .chain(PROTECTED_ABSOLUTE.iter().map(PathBuf::from))
        .collect()
}

pub fn has_unsafe_characters(path: &Path) -> bool {
    path.to_string_lossy()
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | ']'))
}

pub fn protected_paths(
    folder: &Path,
    home: &Path,
    data_dir: &Path,
    other_workspaces: &[PathBuf],
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let candidates = fixed_protected(Some(home))
        .into_iter()
        .chain(std::iter::once(data_dir.to_path_buf()))
        .chain(other_workspaces.iter().cloned());
    for path in candidates {
        if !path.is_absolute() || folder.starts_with(&path) || paths.contains(&path) {
            continue;
        }
        paths.push(path);
    }
    paths
}

fn rule_path(path: &Path) -> String {
    let mut escaped = String::from("/");
    for character in path.to_string_lossy().chars() {
        if matches!(character, '\\' | '*' | '?' | '[' | ']') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn turn_settings(
    folder: &Path,
    temp: &Path,
    home: &Path,
    data_dir: &Path,
    other_workspaces: &[PathBuf],
) -> Value {
    let protected = protected_paths(folder, home, data_dir, other_workspaces);
    let own = rule_path(folder);
    let deny: Vec<String> = protected
        .iter()
        .flat_map(|path| {
            let rule = rule_path(path);
            ["Read", "Edit"]
                .into_iter()
                .flat_map(move |tool| [format!("{tool}({rule})"), format!("{tool}({rule}/**)")])
        })
        .collect();
    let blocked: Vec<String> = protected.iter().map(|path| text(path)).collect();
    json!({
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "autoAllowBashIfSandboxed": false,
            "network": {
                "allowedDomains": [],
                "strictAllowlist": true
            },
            "filesystem": {
                "allowWrite": [text(folder), text(temp)],
                "denyRead": blocked,
                "denyWrite": blocked
            }
        },
        "permissions": {
            "blockReadsOutsideWorkingDirectories": true,
            "ask": ["WebFetch", "WebSearch"],
            "allow": [format!("Read({own}/**)"), format!("Edit({own}/**)")],
            "deny": deny
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(folder: &str, others: &[&str]) -> Value {
        let others: Vec<PathBuf> = others.iter().map(PathBuf::from).collect();
        turn_settings(
            Path::new(folder),
            Path::new("/var/folders/t/T"),
            Path::new("/Users/me"),
            Path::new("/Users/me/Library/Application Support/Shikigami"),
            &others,
        )
    }

    fn strings(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_workspace_turn_gets_the_exact_sandbox_and_permission_settings() {
        let value = settings("/Users/me/Agents/ada", &["/Users/me/Agents/bo"]);
        assert_eq!(
            value["sandbox"],
            json!({
                "enabled": true,
                "failIfUnavailable": true,
                "allowUnsandboxedCommands": false,
                "autoAllowBashIfSandboxed": false,
                "network": {"allowedDomains": [], "strictAllowlist": true},
                "filesystem": {
                    "allowWrite": ["/Users/me/Agents/ada", "/var/folders/t/T"],
                    "denyRead": value["sandbox"]["filesystem"]["denyRead"],
                    "denyWrite": value["sandbox"]["filesystem"]["denyWrite"]
                }
            })
        );
        assert_eq!(
            value["permissions"]["blockReadsOutsideWorkingDirectories"],
            true
        );
        assert_eq!(
            value["permissions"]["ask"],
            json!(["WebFetch", "WebSearch"])
        );
        assert_eq!(
            value["permissions"]["allow"],
            json!([
                "Read(//Users/me/Agents/ada/**)",
                "Edit(//Users/me/Agents/ada/**)"
            ])
        );
        let deny = strings(&value["permissions"]["deny"]);
        for rule in [
            "Read(//Users/me/.ssh)",
            "Read(//Users/me/.ssh/**)",
            "Edit(//Users/me/.ssh/**)",
            "Read(//Users/me/.zshrc)",
            "Edit(//Users/me/.npmrc)",
            "Read(//Users/me/Library/Keychains/**)",
            "Read(//Users/me/Library/Application Support/Google/**)",
            "Read(//Volumes/**)",
            "Edit(//Users/me/.cargo/credentials.toml)",
            "Read(//Users/me/Library/Group Containers/**)",
            "Read(//Users/me/Agents/bo/**)",
            "Edit(//Users/me/Agents/bo/**)",
            "Read(//Users/me/Library/Application Support/Shikigami/**)",
            "Edit(//Users/me/Library/Application Support/Shikigami)",
        ] {
            assert!(deny.contains(&rule), "{rule}");
        }
        assert!(deny
            .iter()
            .all(|rule| rule.starts_with("Read(//") || rule.starts_with("Edit(//")));
    }

    #[test]
    fn the_protected_list_reaches_every_filesystem_and_permission_list() {
        let value = settings("/Users/me/Agents/ada", &["/Users/me/Agents/bo"]);
        let read = strings(&value["sandbox"]["filesystem"]["denyRead"]);
        let write = strings(&value["sandbox"]["filesystem"]["denyWrite"]);
        assert_eq!(read, write);
        assert_eq!(
            read.len(),
            PROTECTED_IN_HOME.len() + PROTECTED_ABSOLUTE.len() + 2
        );
        assert_eq!(strings(&value["permissions"]["deny"]).len(), read.len() * 4);
        for path in [
            "/Users/me/.ssh",
            "/Users/me/.gnupg",
            "/Users/me/.aws",
            "/Users/me/.claude",
            "/Users/me/.claude.json",
            "/Users/me/Library/Keychains",
            "/Users/me/Library/Messages",
            "/Users/me/.zsh_history",
            "/Users/me/.viminfo",
            "/Users/me/.codex",
            "/Volumes",
            "/Users/me/Library/Application Support/Shikigami",
            "/Users/me/Agents/bo",
        ] {
            assert!(read.contains(&path), "{path}");
        }
    }

    #[test]
    fn a_group_turn_in_a_picked_folder_is_confined_the_same_way() {
        let value = settings("/Users/me/Projects/site", &[]);
        assert_eq!(
            value["sandbox"]["filesystem"]["allowWrite"],
            json!(["/Users/me/Projects/site", "/var/folders/t/T"])
        );
        assert_eq!(
            value["permissions"]["allow"],
            json!([
                "Read(//Users/me/Projects/site/**)",
                "Edit(//Users/me/Projects/site/**)"
            ])
        );
        assert_eq!(value["sandbox"]["network"]["allowedDomains"], json!([]));
        assert_eq!(value["sandbox"]["allowUnsandboxedCommands"], json!(false));
    }

    #[test]
    fn the_turn_folder_is_never_denied_even_when_it_equals_or_sits_inside_a_listed_path() {
        let value = settings(
            "/Users/me/Agents/ada",
            &[
                "/Users/me/Agents/ada",
                "/Users/me/Agents",
                "/Users/me/Agents/bo",
            ],
        );
        let read = strings(&value["sandbox"]["filesystem"]["denyRead"]);
        assert!(!read.contains(&"/Users/me/Agents/ada"));
        assert!(!read.contains(&"/Users/me/Agents"));
        assert!(read.contains(&"/Users/me/Agents/bo"));
        let deny = strings(&value["permissions"]["deny"]);
        assert!(deny.iter().all(|rule| !rule.contains("Agents/ada")));
        assert!(deny
            .iter()
            .all(|rule| !rule.contains("(//Users/me/Agents)")));
        assert!(deny
            .iter()
            .all(|rule| !rule.contains("(//Users/me/Agents/**)")));
    }

    #[test]
    fn a_folder_inside_the_data_folder_is_left_open_and_other_paths_stay_denied() {
        let value = settings("/Users/me/Library/Application Support/Shikigami/work", &[]);
        let read = strings(&value["sandbox"]["filesystem"]["denyRead"]);
        assert!(!read.contains(&"/Users/me/Library/Application Support/Shikigami"));
        assert!(read.contains(&"/Users/me/.ssh"));
    }

    #[test]
    fn relative_and_repeated_paths_are_left_out_of_the_protected_list() {
        let others = vec![
            PathBuf::from("relative/dir"),
            PathBuf::from("/Users/me/Agents/bo"),
            PathBuf::from("/Users/me/Agents/bo"),
            PathBuf::from("/Users/me/.ssh"),
        ];
        let paths = protected_paths(
            Path::new("/Users/me/Agents/ada"),
            Path::new("/Users/me"),
            Path::new(""),
            &others,
        );
        assert_eq!(
            paths.len(),
            PROTECTED_IN_HOME.len() + PROTECTED_ABSOLUTE.len() + 1
        );
        assert!(paths.contains(&PathBuf::from("/Users/me/Agents/bo")));
        assert!(!paths.contains(&PathBuf::from("relative/dir")));
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.as_path() == Path::new("/Users/me/.ssh"))
                .count(),
            1
        );
    }

    #[test]
    fn folders_with_pattern_characters_are_unsafe_and_ordinary_ones_are_not() {
        for name in ["/a/b*c", "/a/b?c", "/a/x[", "/a/x]"] {
            assert!(has_unsafe_characters(Path::new(name)), "{name}");
        }
        assert!(!has_unsafe_characters(Path::new("/a/My Docs (1)/x-y_z")));
    }

    #[test]
    fn the_fixed_protected_list_covers_home_entries_and_absolute_volumes() {
        let list = fixed_protected(Some(Path::new("/Users/me")));
        assert!(list.contains(&PathBuf::from("/Users/me/.ssh")));
        assert!(list.contains(&PathBuf::from("/Volumes")));
        assert_eq!(fixed_protected(None), [PathBuf::from("/Volumes")]);
    }

    #[test]
    fn rule_paths_escape_pattern_characters_and_keep_spaces() {
        assert_eq!(
            rule_path(Path::new("/Users/me/a*b[1]/My Docs")),
            "//Users/me/a\\*b\\[1\\]/My Docs"
        );
    }

    #[test]
    fn the_unavailable_text_of_the_cli_is_recognised_in_any_case() {
        assert!(is_unavailable(
            "Error: sandbox required but unavailable: seatbelt is missing"
        ));
        assert!(is_unavailable(
            "  Refusing to start without a working sandbox (see sandbox.failIfUnavailable)."
        ));
        assert!(is_unavailable(
            "Sandbox is required but failed to initialize: no seatbelt"
        ));
        assert!(is_unavailable(
            "Sandbox is required (sandbox.failIfUnavailable) and stopped being available in this session: x"
        ));
        assert!(is_unavailable(
            "These settings did not load, so nothing in them is in effect, including hooks"
        ));
        assert!(is_unavailable(
            "These settings entries were skipped; the rest"
        ));
        assert!(!is_unavailable("The sandbox blocked the write."));
        assert!(!is_unavailable(""));
    }
}
