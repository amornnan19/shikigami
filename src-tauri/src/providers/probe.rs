use std::env;
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

pub async fn run(
    path: &Path,
    args: &[&str],
    search_dirs: &[PathBuf],
    limit: Duration,
) -> Option<ProbeOutput> {
    let search_path = env::join_paths(search_dirs).ok()?;
    let mut command = Command::new(path);
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
