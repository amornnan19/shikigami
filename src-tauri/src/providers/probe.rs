use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

const MAX_OUTPUT_BYTES: u64 = 64 * 1024;

pub struct ProbeOutput {
    pub stdout: String,
}

impl ProbeOutput {
    pub fn first_line(&self) -> Option<String> {
        self.stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    }
}

const ALLOWED_VARIABLES: [&str; 7] = ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "TERM"];
pub const CONFIG_DIR_VARIABLE: &str = "CLAUDE_CONFIG_DIR";
const ALLOWED_PREFIX: &str = "LC_";

pub fn is_allowed_variable(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| ALLOWED_VARIABLES.contains(&name) || name.starts_with(ALLOWED_PREFIX))
}

pub async fn run(
    path: &Path,
    args: &[&str],
    search_dirs: &[PathBuf],
    limit: Duration,
) -> Option<ProbeOutput> {
    run_with(path, args, search_dirs, limit, false, None).await
}

pub async fn run_allowlisted(
    path: &Path,
    args: &[&str],
    search_dirs: &[PathBuf],
    limit: Duration,
) -> Option<ProbeOutput> {
    run_with(path, args, search_dirs, limit, true, None).await
}

pub async fn run_in_config_dir(
    path: &Path,
    args: &[&str],
    search_dirs: &[PathBuf],
    limit: Duration,
    config_dir: &Path,
) -> Option<ProbeOutput> {
    run_with(path, args, search_dirs, limit, true, Some(config_dir)).await
}

async fn run_with(
    path: &Path,
    args: &[&str],
    search_dirs: &[PathBuf],
    limit: Duration,
    allowlisted: bool,
    config_dir: Option<&Path>,
) -> Option<ProbeOutput> {
    let search_path = env::join_paths(search_dirs).ok()?;
    let mut command = Command::new(path);
    if allowlisted {
        command
            .env_clear()
            .envs(std::env::vars_os().filter(|(name, _)| is_allowed_variable(name)));
    }
    if let Some(config_dir) = config_dir {
        command.env(CONFIG_DIR_VARIABLE, config_dir);
    }
    command
        .args(args)
        .env("PATH", search_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = command.spawn().ok()?;
    let group = child.id().and_then(|pid| i32::try_from(pid).ok());
    let mut stdout = child.stdout.take()?;

    let finished = timeout(limit, async {
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(MAX_OUTPUT_BYTES)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        drop(stdout);
        child.wait().await.ok()?;
        Some(ProbeOutput {
            stdout: String::from_utf8_lossy(&bytes).into_owned(),
        })
    })
    .await;

    match finished {
        Ok(output) => output,
        Err(_) => {
            if let Some(group) = group {
                unsafe {
                    libc::killpg(group, libc::SIGKILL);
                }
            }
            let _ = child.wait().await;
            None
        }
    }
}
