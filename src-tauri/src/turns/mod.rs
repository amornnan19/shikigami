mod claude_code;
mod redact;
mod runner;
mod store;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::agents::now_millis;
use crate::api_key;
use crate::db::Database;
use crate::providers::{self, CLAUDE_CODE_ID};
use store::{BeginError, ConversationPage, StepView};

pub const EVENT_NAME: &str = "turn-event";
pub const MESSAGE_MAX_CHARS: usize = 100_000;
const EXIT_TERM_GRACE: Duration = Duration::from_secs(2);
const EXIT_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Serialize)]
pub struct TurnError {
    pub code: &'static str,
    pub message: String,
}

impl TurnError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn database(error: impl std::fmt::Display) -> Self {
        Self::new(
            "database-failed",
            format!("Could not save the message: {error}"),
        )
    }

    fn unavailable() -> Self {
        Self::new("database-failed", "The app database is not available.")
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum TurnEvent {
    Started,
    TextDelta {
        item_id: String,
        text: String,
    },
    TextSet {
        item_id: String,
        text: String,
    },
    StepStarted {
        step: StepView,
    },
    StepEnded {
        step_id: String,
        status: String,
        detail: Option<String>,
    },
    Notice {
        item_id: String,
        text: String,
    },
    Ended {
        status: String,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnEnvelope {
    pub agent_id: String,
    pub turn_id: String,
    #[serde(flatten)]
    pub event: TurnEvent,
}

pub trait EventSink: Send + Sync {
    fn emit(&self, envelope: &TurnEnvelope);
}

impl EventSink for AppHandle {
    fn emit(&self, envelope: &TurnEnvelope) {
        let _ = Emitter::emit(self, EVENT_NAME, envelope);
    }
}

#[derive(Default)]
pub struct TurnRegistry(Mutex<HashMap<String, i32>>);

impl TurnRegistry {
    fn groups(&self) -> std::sync::MutexGuard<'_, HashMap<String, i32>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_group(&self, agent_id: &str, group: i32) {
        self.groups().insert(agent_id.to_owned(), group);
    }

    fn clear(&self, agent_id: &str) {
        self.groups().remove(agent_id);
    }

    fn kill(&self, agent_id: &str) {
        if let Some(group) = self.groups().get(agent_id).copied() {
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }

    pub fn kill_all(&self, grace: Duration) {
        let groups: Vec<i32> = std::mem::take(&mut *self.groups()).into_values().collect();
        for group in &groups {
            unsafe {
                libc::killpg(*group, libc::SIGTERM);
            }
        }
        let deadline = Instant::now() + grace;
        let gone = |group: &i32| unsafe { libc::killpg(*group, 0) } != 0;
        while Instant::now() < deadline && !groups.iter().all(gone) {
            std::thread::sleep(EXIT_POLL);
        }
        for group in groups.iter().filter(|group| !gone(group)) {
            unsafe {
                libc::killpg(*group, libc::SIGKILL);
            }
        }
    }
}

pub fn stop_all_turns(app: &AppHandle) {
    if let Some(registry) = app.try_state::<TurnRegistry>() {
        registry.kill_all(EXIT_TERM_GRACE);
    }
}

pub struct Launch {
    pub agent_id: String,
    pub turn_id: String,
    pub executable: PathBuf,
    pub search_dirs: Vec<PathBuf>,
    pub workspace: PathBuf,
    pub model: String,
    pub session: claude_code::Session,
    pub text: String,
    pub started_at: i64,
    pub environment: Vec<(OsString, OsString)>,
}

fn prepare(
    db: &Database,
    dirs: &[PathBuf],
    agent_id: &str,
    text: &str,
) -> Result<Launch, TurnError> {
    if text.trim().is_empty() {
        return Err(TurnError::new("empty", "Enter a message."));
    }
    if text.chars().count() > MESSAGE_MAX_CHARS {
        return Err(TurnError::new(
            "too-long",
            format!("Use at most {MESSAGE_MAX_CHARS} characters."),
        ));
    }
    let agent = db
        .with_connection(|connection| store::agent(connection, agent_id))
        .map_err(TurnError::database)?
        .ok_or_else(|| TurnError::new("not-found", "This agent no longer exists."))?;
    let executable = (agent.provider == CLAUDE_CODE_ID)
        .then(|| providers::executable_path(&agent.provider, dirs))
        .flatten()
        .ok_or_else(|| {
            TurnError::new(
                "provider-unavailable",
                "This agent's provider is not available.",
            )
        })?;
    let started_at = now_millis();
    let begun = db
        .with_connection(|connection| store::begin_turn(connection, agent_id, text, started_at))
        .map_err(|error| match error {
            BeginError::NotFound => TurnError::new("not-found", "This agent no longer exists."),
            BeginError::Busy => TurnError::new("busy", "This agent is already working."),
            BeginError::Database(error) => TurnError::database(error),
        })?;
    Ok(Launch {
        agent_id: agent_id.to_owned(),
        turn_id: begun.turn_id,
        executable,
        search_dirs: dirs.to_vec(),
        workspace: PathBuf::from(begun.agent.workspace_path),
        model: begun.agent.model,
        session: begun.session,
        text: text.to_owned(),
        started_at,
        environment: std::env::vars_os().collect(),
    })
}

pub fn sweep_interrupted(db: &Database) -> Result<usize, String> {
    db.with_connection(|connection| store::sweep_interrupted(connection, now_millis()))
        .map_err(|error| format!("could not recover running turns: {error}"))
}

async fn drive(app: AppHandle, launch: Launch) {
    let key_app = app.clone();
    let key = tauri::async_runtime::spawn_blocking(move || {
        api_key::api_key_for_turn(&key_app, CLAUDE_CODE_ID)
    })
    .await;
    let key = match key {
        Ok(read) => runner::ApiKey::from_read(read),
        Err(_) => runner::ApiKey::Unreadable,
    };
    let (Some(db), Some(registry)) = (app.try_state::<Database>(), app.try_state::<TurnRegistry>())
    else {
        return;
    };
    runner::run(&db, &app, &registry, launch, key).await;
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    agent_id: String,
    text: String,
) -> Result<SendResult, TurnError> {
    let prepare_app = app.clone();
    let launch = tauri::async_runtime::spawn_blocking(move || {
        let db = prepare_app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        prepare(&db, &providers::search_dirs(), &agent_id, &text)
    })
    .await
    .map_err(TurnError::database)??;
    let turn_id = launch.turn_id.clone();
    tauri::async_runtime::spawn(drive(app, launch));
    Ok(SendResult { turn_id })
}

#[tauri::command]
pub async fn get_conversation(
    app: AppHandle,
    agent_id: String,
    before_turn_id: Option<String>,
) -> Result<ConversationPage, TurnError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        db.with_connection(|connection| {
            store::conversation(connection, &agent_id, before_turn_id.as_deref())
        })
        .map_err(TurnError::database)
    })
    .await
    .map_err(TurnError::database)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        _data: tempfile::TempDir,
        bin: tempfile::TempDir,
        db: Database,
    }

    impl Fixture {
        fn new(provider: &str) -> Self {
            let data = tempfile::tempdir().unwrap();
            let bin = tempfile::tempdir().unwrap();
            let claude = bin.path().join("claude");
            fs::write(&claude, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
            let db = Database::open(data.path()).unwrap();
            db.with_connection(|c| {
                c.execute(
                    "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                     VALUES ('a', 'Ada', 'red', ?1, 'haiku', '/w', 1, 1)",
                    [provider],
                )
                .unwrap();
            });
            Self {
                _data: data,
                bin,
                db,
            }
        }

        fn send(&self, agent: &str, text: &str) -> Result<Launch, TurnError> {
            prepare(&self.db, &[self.bin.path().to_path_buf()], agent, text)
        }

        fn turns(&self) -> i64 {
            self.db
                .with_connection(|c| c.query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0)))
                .unwrap()
        }
    }

    fn code(result: Result<Launch, TurnError>) -> &'static str {
        result.err().expect("expected an error").code
    }

    #[test]
    fn a_valid_message_starts_a_turn_for_the_agents_workspace_and_model() {
        let fx = Fixture::new("claude-code");
        let launch = fx.send("a", "hello").unwrap();
        assert_eq!(launch.workspace, PathBuf::from("/w"));
        assert_eq!(launch.model, "haiku");
        assert_eq!(launch.executable, fx.bin.path().join("claude"));
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn empty_and_too_long_messages_start_nothing() {
        let fx = Fixture::new("claude-code");
        assert_eq!(code(fx.send("a", "  \n ")), "empty");
        assert_eq!(
            code(fx.send("a", &"x".repeat(MESSAGE_MAX_CHARS + 1))),
            "too-long"
        );
        assert!(fx.send("a", &"ก".repeat(MESSAGE_MAX_CHARS)).is_ok());
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn a_running_turn_makes_the_next_send_busy() {
        let fx = Fixture::new("claude-code");
        fx.send("a", "one").unwrap();
        assert_eq!(code(fx.send("a", "two")), "busy");
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn an_unknown_agent_and_an_unsupported_provider_are_refused() {
        let fx = Fixture::new("codex");
        assert_eq!(code(fx.send("missing", "hi")), "not-found");
        assert_eq!(code(fx.send("a", "hi")), "provider-unavailable");
        assert_eq!(fx.turns(), 0);
    }

    #[test]
    fn events_serialise_with_a_kebab_kind_and_camel_case_fields() {
        let envelope = TurnEnvelope {
            agent_id: "a".into(),
            turn_id: "t".into(),
            event: TurnEvent::TextDelta {
                item_id: "i".into(),
                text: "x".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&envelope).unwrap(),
            serde_json::json!({"agentId":"a","turnId":"t","kind":"text-delta","itemId":"i","text":"x"})
        );
        let ended = TurnEnvelope {
            agent_id: "a".into(),
            turn_id: "t".into(),
            event: TurnEvent::Ended {
                status: "failed".into(),
                error: Some("x".into()),
            },
        };
        assert_eq!(
            serde_json::to_value(&ended).unwrap()["kind"],
            serde_json::json!("ended")
        );
    }

    fn spawn_group(script: &str) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        std::process::Command::new("/bin/sh")
            .args(["-c", script])
            .process_group(0)
            .spawn()
            .unwrap()
    }

    fn group_of(child: &std::process::Child) -> i32 {
        i32::try_from(child.id()).unwrap()
    }

    #[test]
    fn exit_kills_every_registered_group_with_a_term_signal_first() {
        use std::os::unix::process::ExitStatusExt;
        let registry = TurnRegistry::default();
        let mut first = spawn_group("sleep 30");
        let mut second = spawn_group("sleep 30");
        registry.set_group("a", group_of(&first));
        registry.set_group("b", group_of(&second));
        let started = Instant::now();
        let reaper = std::thread::spawn(move || (first.wait().unwrap(), second.wait().unwrap()));
        registry.kill_all(Duration::from_secs(5));
        let (first, second) = reaper.join().unwrap();
        assert_eq!(first.signal(), Some(libc::SIGTERM));
        assert_eq!(second.signal(), Some(libc::SIGTERM));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(registry.groups().is_empty());
    }

    #[test]
    fn exit_kills_a_group_that_ignores_the_term_signal_after_the_grace() {
        use std::os::unix::process::ExitStatusExt;
        let registry = TurnRegistry::default();
        let child = spawn_group("trap '' TERM; while true; do sleep 1; done");
        registry.set_group("a", group_of(&child));
        let reaper = std::thread::spawn(move || {
            let mut child = child;
            child.wait().unwrap()
        });
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        registry.kill_all(Duration::from_millis(500));
        let status = reaper.join().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert!(started.elapsed() >= Duration::from_millis(500));
    }
}
