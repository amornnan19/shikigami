mod relay;
mod session;
pub mod store;

use std::ffi::OsString;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use crate::db::Database;
use crate::turns::TurnRegistry;

#[cfg(test)]
pub use session::abort_last_accepter;
pub use session::{socket_dir, Channel, Inbound};
pub use store::{group_waiting, ApprovalView};

pub const RELAY_FLAG: &str = "--approval-relay";
pub const SERVER_NAME: &str = "shikigami";
pub const TOOL_NAME: &str = "approve";
pub const PROMPT_TOOL: &str = "mcp__shikigami__approve";
pub const CALL_TIMEOUT_MS: u64 = 1_800_000;
pub const MAX_OPEN: usize = 32;
pub const DENY_USER: &str = "The user denied this request.";
pub const DENY_TIMEOUT: &str = "No answer within 10 minutes, so this request was denied.";
pub const DENY_STOP: &str = "The turn stopped, so this request was denied.";
pub const DENY_INTERRUPTED: &str = "The request was interrupted, so it was denied.";
pub const DENY_SHUTDOWN: &str = "Shikigami was closing, so this request was denied.";
pub const DENY_UNREACHABLE: &str = "Shikigami could not ask the user, so this request was denied.";
pub const DENY_BUSY: &str = "Too many requests are waiting for the user, so this one was denied.";
const UPDATED_INPUT_LIMIT: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Answer { approval_id: String, allow: bool },
    Stop,
    Shutdown,
}

#[derive(Debug, Serialize)]
pub struct ApprovalError {
    pub code: &'static str,
    pub message: String,
}

impl ApprovalError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn allow_reply(input: &Value) -> Value {
    if input.to_string().len() > UPDATED_INPUT_LIMIT {
        json!({"behavior": "allow"})
    } else {
        json!({"behavior": "allow", "updatedInput": input})
    }
}

pub fn deny_reply(message: &str) -> Value {
    json!({"behavior": "deny", "message": message})
}

pub fn relay_config(executable: &std::path::Path, socket: &std::path::Path, token: &str) -> String {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "command": executable,
                "args": [RELAY_FLAG, socket, token],
                "timeout": CALL_TIMEOUT_MS
            }
        }
    })
    .to_string()
}

pub fn answer(
    db: &Database,
    registry: &TurnRegistry,
    approval_id: &str,
    allow: bool,
) -> Result<(), ApprovalError> {
    if registry.is_closing() {
        return Err(ApprovalError::new("shutting-down", "Shikigami is closing."));
    }
    let target = db
        .with_connection(|connection| store::target(connection, approval_id))
        .map_err(|error| {
            ApprovalError::new(
                "database-failed",
                format!("Could not read the request: {error}"),
            )
        })?
        .ok_or_else(|| ApprovalError::new("not-found", "This request no longer exists."))?;
    if target.status != "pending" {
        return Err(ApprovalError::new(
            "already-answered",
            "This request was already answered.",
        ));
    }
    if !registry.send_signal(
        &target.turn_id,
        Message::Answer {
            approval_id: approval_id.to_owned(),
            allow,
        },
    ) {
        return Err(ApprovalError::new(
            "not-running",
            "This request can no longer be answered.",
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn answer_approval(
    db: State<Database>,
    registry: State<TurnRegistry>,
    approval_id: String,
    allow: bool,
) -> Result<(), ApprovalError> {
    answer(&db, &registry, &approval_id, allow)
}

pub fn run_relay(arguments: impl IntoIterator<Item = OsString>) -> Option<i32> {
    let mut arguments = arguments.into_iter().skip(1);
    if arguments.next()? != RELAY_FLAG {
        return None;
    }
    let (Some(socket), Some(token)) = (arguments.next(), arguments.next()) else {
        return Some(2);
    };
    Some(relay::run(&PathBuf::from(socket), &token.to_string_lossy()))
}

pub use relay::serve as serve_relay;

#[cfg(test)]
mod tests;
