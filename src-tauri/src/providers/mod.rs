mod claude_code;
mod codex;
mod probe;
mod search;

use probe::ProbeOutput;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

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
    let (version, sign_in) = tokio::join!(
        probe::run(&path, &["--version"], limit),
        probe::run(&path, spec.sign_in_args, limit),
    );
    status.installed = true;
    status.path = Some(path.to_string_lossy().into_owned());
    status.version = version.and_then(|output| output.first_line());
    status.signed_in = sign_in
        .map(|output| (spec.interpret_sign_in)(&output))
        .unwrap_or(SignedIn::Unknown);
    status
}

async fn detect_all(dirs: &[PathBuf], limit: Duration) -> Vec<ProviderStatus> {
    let (claude, codex) = tokio::join!(
        detect(&claude_code::SPEC, dirs, limit),
        detect(&codex::SPEC, dirs, limit),
    );
    vec![claude, codex]
}

#[tauri::command]
pub async fn list_providers() -> Result<Vec<ProviderStatus>, String> {
    let dirs = tauri::async_runtime::spawn_blocking(search::search_dirs)
        .await
        .map_err(|error| error.to_string())?;
    Ok(detect_all(&dirs, PROBE_TIMEOUT).await)
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
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
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
        let all = detect_all(&dirs(&dir), LIMIT).await;
        assert_eq!(all.len(), 2);
        for status in all {
            assert!(!status.installed);
            assert_eq!(status.path, None);
            assert_eq!(status.version, None);
            assert_eq!(status.signed_in, SignedIn::Unknown);
            assert!(status.install_hint.starts_with("https://"));
        }
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
    async fn codex_exit_code_decides_sign_in() {
        let out = tempfile::tempdir().unwrap();
        fake(
            out.path(),
            "codex",
            r#"case "$1" in
  --version) echo "codex-cli 0.1.0" ;;
  login) exit 1 ;;
esac"#,
        );
        let status = detect(&codex::SPEC, &dirs(&out), LIMIT).await;
        assert!(status.installed);
        assert_eq!(status.version.as_deref(), Some("codex-cli 0.1.0"));
        assert_eq!(status.signed_in, SignedIn::No);

        let signed_in = tempfile::tempdir().unwrap();
        fake(
            signed_in.path(),
            "codex",
            r#"case "$1" in
  --version) echo "codex-cli 0.1.0" ;;
  login) echo "Logged in" ;;
esac"#,
        );
        let status = detect(&codex::SPEC, &dirs(&signed_in), LIMIT).await;
        assert_eq!(status.signed_in, SignedIn::Yes);
    }

    #[tokio::test]
    async fn slow_probe_gives_unknown_within_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        fake(dir.path(), "claude", "exec sleep 5");
        let started = Instant::now();
        let status = detect(&claude_code::SPEC, &dirs(&dir), Duration::from_millis(200)).await;
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(status.installed);
        assert_eq!(status.version, None);
        assert_eq!(status.signed_in, SignedIn::Unknown);
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
