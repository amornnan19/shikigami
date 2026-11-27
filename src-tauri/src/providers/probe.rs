use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

pub struct ProbeOutput {
    pub success: bool,
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

pub async fn run(path: &Path, args: &[&str], limit: Duration) -> Option<ProbeOutput> {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = timeout(limit, command.output()).await.ok()?.ok()?;
    Some(ProbeOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}
