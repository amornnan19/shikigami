mod claude_code;
mod probe;
mod search;

pub use probe::is_allowed_variable;
use probe::ProbeOutput;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const CLAUDE_CODE_ID: &str = claude_code::SPEC.id;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SignedIn {
    Yes,
    No,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub signed_in: SignedIn,
    pub install_hint: &'static str,
    pub sign_in_hint: &'static str,
}

pub struct ProviderSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub executable: &'static str,
    pub install_hint: &'static str,
    pub sign_in_hint: &'static str,
    pub sign_in_args: &'static [&'static str],
    pub version_prefix: &'static str,
    pub sign_in_floor: [u32; 3],
    pub interpret_sign_in: fn(&ProbeOutput) -> SignedIn,
}

async fn detect(spec: &ProviderSpec, dirs: &[PathBuf], limit: Duration) -> ProviderStatus {
    let mut status = ProviderStatus {
        id: spec.id,
        name: spec.name,
        installed: false,
        path: None,
        version: None,
        signed_in: SignedIn::Unknown,
        install_hint: spec.install_hint,
        sign_in_hint: spec.sign_in_hint,
    };
    let Some(path) = search::find_executable(spec.executable, dirs) else {
        return status;
    };
    status.installed = true;
    status.path = Some(path.to_string_lossy().into_owned());
    status.version = probe::run(&path, &["--version"], dirs, limit)
        .await
        .and_then(|output| output.first_line());
    let supports_sign_in = status
        .version
        .as_deref()
        .and_then(|line| parse_version(line, spec.version_prefix))
        .is_some_and(|version| version >= spec.sign_in_floor);
    if supports_sign_in {
        status.signed_in = probe::run(&path, spec.sign_in_args, dirs, limit)
            .await
            .map(|output| (spec.interpret_sign_in)(&output))
            .unwrap_or(SignedIn::Unknown);
    }
    status
}

fn parse_version(line: &str, prefix: &str) -> Option<[u32; 3]> {
    let token = line.strip_prefix(prefix)?.split_whitespace().next()?;
    let mut parts = token.splitn(3, '.');
    let mut next = || {
        let part = parts.next()?;
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    Some([next()?, next()?, next()?])
}

static FLAG_SUPPORT: OnceLock<Mutex<HashMap<(PathBuf, String), bool>>> = OnceLock::new();

pub async fn help_lists_flag(executable: &Path, dirs: &[PathBuf], flag: &str) -> bool {
    help_lists_flag_within(executable, dirs, flag, PROBE_TIMEOUT).await
}

async fn help_lists_flag_within(
    executable: &Path,
    dirs: &[PathBuf],
    flag: &str,
    limit: Duration,
) -> bool {
    let cache = FLAG_SUPPORT.get_or_init(Mutex::default);
    let key = (executable.to_path_buf(), flag.to_owned());
    let known = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
        .copied();
    if let Some(listed) = known {
        return listed;
    }
    let Some(output) = probe::run_allowlisted(executable, &["--help"], dirs, limit).await else {
        return false;
    };
    let listed = output.stdout.contains(flag);
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key, listed);
    listed
}

static VERSIONS: OnceLock<Mutex<HashMap<PathBuf, [u32; 3]>>> = OnceLock::new();

pub async fn version_at_least(executable: &Path, dirs: &[PathBuf], minimum: [u32; 3]) -> bool {
    version_at_least_within(executable, dirs, minimum, PROBE_TIMEOUT).await
}

async fn version_at_least_within(
    executable: &Path,
    dirs: &[PathBuf],
    minimum: [u32; 3],
    limit: Duration,
) -> bool {
    let cache = VERSIONS.get_or_init(Mutex::default);
    let known = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(executable)
        .copied();
    let version = match known {
        Some(version) => version,
        None => {
            let Some(output) =
                probe::run_allowlisted(executable, &["--version"], dirs, limit).await
            else {
                return false;
            };
            let Some(version) = output
                .first_line()
                .and_then(|line| parse_version(&line, claude_code::SPEC.version_prefix))
            else {
                return false;
            };
            cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(executable.to_path_buf(), version);
            version
        }
    };
    version >= minimum
}

pub async fn claude_status(dirs: &[PathBuf]) -> ProviderStatus {
    detect(&claude_code::SPEC, dirs, PROBE_TIMEOUT).await
}

pub fn is_known(id: &str) -> bool {
    id == CLAUDE_CODE_ID
}

pub fn name_of(id: &str) -> &'static str {
    if is_known(id) {
        claude_code::SPEC.name
    } else {
        "The provider"
    }
}

pub fn is_installed(id: &str, dirs: &[PathBuf]) -> bool {
    executable_path(id, dirs).is_some()
}

pub fn executable_path(id: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    is_known(id)
        .then(|| search::find_executable(claude_code::SPEC.executable, dirs))
        .flatten()
}

pub fn search_dirs() -> Vec<PathBuf> {
    search::search_dirs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Instant;

    const LIMIT: Duration = Duration::from_secs(5);

    fn fake(dir: &Path, name: &str, body: &str) {
        fake_with_shebang(dir, name, "/bin/sh", body);
    }

    fn fake_with_shebang(dir: &Path, name: &str, interpreter: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!{interpreter}\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn dirs(dir: &tempfile::TempDir) -> Vec<PathBuf> {
        vec![dir.path().to_path_buf()]
    }

    const CLAUDE_SIGNED_IN: &str = r#"case "$1" in
  --version) echo "2.1.332 (Claude Code)" ;;
  auth) echo '{"loggedIn": true, "authMethod": "claude.ai"}' ;;
esac"#;

    #[tokio::test]
    async fn missing_executable_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        assert!(!status.installed);
        assert_eq!(status.path, None);
        assert_eq!(status.version, None);
        assert_eq!(status.signed_in, SignedIn::Unknown);
        assert!(status.install_hint.starts_with("https://"));
    }

    #[tokio::test]
    async fn signed_in_claude_is_ready_with_version() {
        let dir = tempfile::tempdir().unwrap();
        fake(dir.path(), "claude", CLAUDE_SIGNED_IN);
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        assert!(status.installed);
        assert_eq!(status.version.as_deref(), Some("2.1.332 (Claude Code)"));
        assert_eq!(status.signed_in, SignedIn::Yes);
        assert_eq!(
            status.path,
            Some(dir.path().join("claude").to_string_lossy().into_owned())
        );
    }

    #[tokio::test]
    async fn signed_out_claude_is_no() {
        let dir = tempfile::tempdir().unwrap();
        fake(
            dir.path(),
            "claude",
            r#"case "$1" in
  --version) echo "2.1.332 (Claude Code)" ;;
  auth) echo '{"loggedIn": false}'; exit 1 ;;
esac"#,
        );
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        assert!(status.installed);
        assert_eq!(status.signed_in, SignedIn::No);
    }

    #[tokio::test]
    async fn slow_probe_gives_unknown_within_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        fake(dir.path(), "claude", "exec /bin/sleep 5");
        let started = Instant::now();
        let status = detect(&claude_code::SPEC, &dirs(&dir), Duration::from_millis(200)).await;
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(status.installed);
        assert_eq!(status.version, None);
        assert_eq!(status.signed_in, SignedIn::Unknown);
    }

    fn process_exists(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    const KILL_PROBE_LIMIT: Duration = Duration::from_secs(4);
    const LOAD_SLACK: Duration = Duration::from_secs(10);
    const TIMER_SKEW: Duration = Duration::from_millis(50);

    fn read_pid(path: &Path) -> Option<i32> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    async fn wait_for_pid(path: &Path) -> i32 {
        let deadline = Instant::now() + LOAD_SLACK;
        loop {
            if let Some(pid) = read_pid(path) {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "{} was not written",
                path.display()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let shell_pid = dir.path().join("shell.pid");
        let child_pid = dir.path().join("child.pid");
        fake(
            dir.path(),
            "claude",
            &format!(
                "echo $$ > {shell}\n/bin/sleep 60 &\necho $! > {child}\nwait",
                shell = shell_pid.display(),
                child = child_pid.display()
            ),
        );
        let started = Instant::now();
        let status = detect(&claude_code::SPEC, &dirs(&dir), KILL_PROBE_LIMIT).await;
        assert!(started.elapsed() + TIMER_SKEW >= KILL_PROBE_LIMIT);
        assert!(started.elapsed() < KILL_PROBE_LIMIT + LOAD_SLACK);
        assert_eq!(status.version, None);
        let pids = [
            wait_for_pid(&shell_pid).await,
            wait_for_pid(&child_pid).await,
        ];
        let deadline = Instant::now() + LOAD_SLACK;
        while pids.iter().any(|pid| process_exists(*pid)) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        for pid in pids {
            assert!(!process_exists(pid), "process {pid} is still running");
        }
    }

    #[tokio::test]
    async fn a_flag_is_listed_when_the_help_names_it_and_the_answer_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("calls");
        fake(
            dir.path(),
            "claude",
            &format!(
                "echo x >> {}\necho '  --system-prompt-snapshot <on|off>'",
                counter.display()
            ),
        );
        let path = dir.path().join("claude");
        assert!(help_lists_flag(&path, &dirs(&dir), "--system-prompt-snapshot").await);
        assert!(help_lists_flag(&path, &dirs(&dir), "--system-prompt-snapshot").await);
        assert!(!help_lists_flag(&path, &dirs(&dir), "--other-flag").await);
        assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 2);
    }

    #[tokio::test]
    async fn the_help_probe_runs_with_the_allowlisted_environment_only() {
        std::env::set_var("SHIKIGAMI_PROBE_LEAK_CHECK", "leaked");
        let dir = tempfile::tempdir().unwrap();
        fake(
            dir.path(),
            "claude",
            "if [ -z \"$SHIKIGAMI_PROBE_LEAK_CHECK\" ]; then echo '--clean-environment'; fi",
        );
        let path = dir.path().join("claude");
        assert!(help_lists_flag(&path, &dirs(&dir), "--clean-environment").await);
        std::env::remove_var("SHIKIGAMI_PROBE_LEAK_CHECK");
    }

    #[tokio::test]
    async fn a_help_probe_that_times_out_is_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let slow = dir.path().join("slow");
        fake(
            dir.path(),
            "claude",
            &format!(
                "if [ -f {m} ]; then /bin/sleep 30; fi\necho '--late-flag'",
                m = slow.display()
            ),
        );
        fs::write(&slow, "").unwrap();
        let path = dir.path().join("claude");
        let short = Duration::from_millis(400);
        assert!(!help_lists_flag_within(&path, &dirs(&dir), "--late-flag", short).await);
        fs::remove_file(&slow).unwrap();
        assert!(help_lists_flag_within(&path, &dirs(&dir), "--late-flag", LIMIT).await);
    }

    #[tokio::test]
    async fn output_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        fake(
            dir.path(),
            "claude",
            "exec /usr/bin/yes xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        );
        let started = Instant::now();
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(status.signed_in, SignedIn::Unknown);
    }

    #[tokio::test]
    async fn cli_can_find_its_interpreter_in_the_search_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fake(dir.path(), "fakeinterp", "exec /bin/sh \"$@\"");
        fake_with_shebang(
            dir.path(),
            "claude",
            "/usr/bin/env fakeinterp",
            CLAUDE_SIGNED_IN,
        );
        assert!(std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .all(|entry| !entry.join("fakeinterp").exists()));
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        assert_eq!(status.version.as_deref(), Some("2.1.332 (Claude Code)"));
        assert_eq!(status.signed_in, SignedIn::Yes);
    }

    #[tokio::test]
    async fn old_versions_do_not_get_the_sign_in_probe() {
        let cases = [("claude", "1.0.100 (Claude Code)", &claude_code::SPEC)];
        for (name, version, spec) in cases {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("sign-in-probed");
            fake(
                dir.path(),
                name,
                &format!(
                    "case \"$1\" in\n  --version) echo '{version}' ;;\n  *) /usr/bin/touch {marker}; echo '{{\"loggedIn\": true}}' ;;\nesac",
                    marker = marker.display()
                ),
            );
            let status = detect(spec, &dirs(&dir), LIMIT).await;
            assert!(status.installed);
            assert_eq!(status.version.as_deref(), Some(version));
            assert_eq!(status.signed_in, SignedIn::Unknown, "{name} {version}");
            assert!(!marker.exists(), "{name} {version} was probed");
        }
    }

    #[test]
    fn parses_versions() {
        assert_eq!(
            parse_version("2.1.332 (Claude Code)", ""),
            Some([2, 1, 332])
        );
        assert_eq!(
            parse_version("v2.1.0-alpha.1 (Claude Code)", "v"),
            Some([2, 1, 0])
        );
        assert_eq!(parse_version("v2.1", "v"), None);
        assert_eq!(parse_version("hello", ""), None);
    }

    #[tokio::test]
    async fn garbage_output_gives_unknown() {
        let dir = tempfile::tempdir().unwrap();
        fake(dir.path(), "claude", "echo 'not json at all'");
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        assert!(status.installed);
        assert_eq!(status.signed_in, SignedIn::Unknown);

        let empty = tempfile::tempdir().unwrap();
        fake(empty.path(), "claude", "exit 0");
        let status = detect(&claude_code::SPEC, &dirs(&empty), LIMIT).await;
        assert_eq!(status.version, None);
        assert_eq!(status.signed_in, SignedIn::Unknown);
    }

    #[tokio::test]
    async fn status_serialises_in_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        fake(dir.path(), "claude", CLAUDE_SIGNED_IN);
        let status = detect(&claude_code::SPEC, &dirs(&dir), LIMIT).await;
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["signedIn"], "yes");
        assert_eq!(value["installHint"], claude_code::SPEC.install_hint);
        assert_eq!(value["signInHint"], "claude auth login");
    }
}
