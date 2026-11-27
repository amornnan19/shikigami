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
use store::{BeginError, ConversationPage, QueuedMessage, Sent, StepView};

pub const EVENT_NAME: &str = "turn-event";
pub const QUEUE_EVENT_NAME: &str = "queue-changed";
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
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SendResult {
    Started { turn_id: String },
    Queued { queued_id: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueueChanged<'a> {
    agent_id: &'a str,
}

fn emit_queue_changed(app: &AppHandle, agent_id: &str) {
    let _ = Emitter::emit(app, QUEUE_EVENT_NAME, QueueChanged { agent_id });
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
struct Slot {
    group: Option<i32>,
    stop_requested: bool,
}

#[derive(Default)]
pub struct TurnRegistry(Mutex<HashMap<String, Slot>>);

impl TurnRegistry {
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn begin(&self, agent_id: &str) {
        self.slots().insert(agent_id.to_owned(), Slot::default());
    }

    fn set_group(&self, agent_id: &str, group: i32) {
        let mut slots = self.slots();
        let slot = slots.entry(agent_id.to_owned()).or_default();
        slot.group = Some(group);
        if slot.stop_requested {
            unsafe {
                libc::killpg(group, libc::SIGTERM);
            }
        }
    }

    fn clear_group(&self, agent_id: &str) {
        if let Some(slot) = self.slots().get_mut(agent_id) {
            slot.group = None;
        }
    }

    fn clear(&self, agent_id: &str) {
        self.slots().remove(agent_id);
    }

    fn stop_requested(&self, agent_id: &str) -> bool {
        self.slots()
            .get(agent_id)
            .is_some_and(|slot| slot.stop_requested)
    }

    fn request_stop(&self, agent_id: &str) -> bool {
        let mut slots = self.slots();
        let Some(slot) = slots.get_mut(agent_id) else {
            return false;
        };
        slot.stop_requested = true;
        if let Some(group) = slot.group {
            unsafe {
                libc::killpg(group, libc::SIGTERM);
            }
        }
        true
    }

    fn kill_stopping(&self, agent_id: &str) {
        if let Some(group) = self
            .slots()
            .get(agent_id)
            .filter(|slot| slot.stop_requested)
            .and_then(|slot| slot.group)
        {
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }

    fn kill(&self, agent_id: &str) {
        if let Some(group) = self.slots().get(agent_id).and_then(|slot| slot.group) {
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }

    pub fn kill_all(&self, grace: Duration) {
        let groups: Vec<i32> = std::mem::take(&mut *self.slots())
            .into_values()
            .filter_map(|slot| slot.group)
            .collect();
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

async fn escalate_stop(registry: &TurnRegistry, agent_id: &str, grace: Duration) {
    tokio::time::sleep(grace).await;
    registry.kill_stopping(agent_id);
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

fn validate(text: &str) -> Result<(), TurnError> {
    if text.trim().is_empty() {
        return Err(TurnError::new("empty", "Enter a message."));
    }
    if text.chars().count() > MESSAGE_MAX_CHARS {
        return Err(TurnError::new(
            "too-long",
            format!("Use at most {MESSAGE_MAX_CHARS} characters."),
        ));
    }
    Ok(())
}

fn executable_of(db: &Database, dirs: &[PathBuf], agent_id: &str) -> Result<PathBuf, TurnError> {
    let agent = db
        .with_connection(|connection| store::agent(connection, agent_id))
        .map_err(TurnError::database)?
        .ok_or_else(|| TurnError::new("not-found", "This agent no longer exists."))?;
    (agent.provider == CLAUDE_CODE_ID)
        .then(|| providers::executable_path(&agent.provider, dirs))
        .flatten()
        .ok_or_else(|| {
            TurnError::new(
                "provider-unavailable",
                "This agent's provider is not available.",
            )
        })
}

fn launch_of(
    registry: &TurnRegistry,
    dirs: &[PathBuf],
    agent_id: &str,
    executable: PathBuf,
    begun: store::Begun,
    text: String,
    started_at: i64,
) -> Launch {
    registry.begin(agent_id);
    Launch {
        agent_id: agent_id.to_owned(),
        turn_id: begun.turn_id,
        executable,
        search_dirs: dirs.to_vec(),
        workspace: PathBuf::from(begun.agent.workspace_path),
        model: begun.agent.model,
        session: begun.session,
        text,
        started_at,
        environment: std::env::vars_os().collect(),
    }
}

enum Prepared {
    Started(Launch),
    Queued(QueuedMessage),
}

fn prepare(
    db: &Database,
    registry: &TurnRegistry,
    dirs: &[PathBuf],
    agent_id: &str,
    text: &str,
) -> Result<Prepared, TurnError> {
    validate(text)?;
    let executable = executable_of(db, dirs, agent_id)?;
    let started_at = now_millis();
    let sent = db
        .with_connection(|connection| store::begin_or_queue(connection, agent_id, text, started_at))
        .map_err(|error| match error {
            BeginError::NotFound => TurnError::new("not-found", "This agent no longer exists."),
            BeginError::Busy => TurnError::new("busy", "This agent is already working."),
            BeginError::Database(error) => TurnError::database(error),
        })?;
    Ok(match sent {
        Sent::Started(begun) => Prepared::Started(launch_of(
            registry,
            dirs,
            agent_id,
            executable,
            begun,
            text.to_owned(),
            started_at,
        )),
        Sent::Queued(queued) => Prepared::Queued(queued),
    })
}

fn start_next(
    db: &Database,
    registry: &TurnRegistry,
    dirs: &[PathBuf],
    agent_id: &str,
) -> Option<Launch> {
    let executable = executable_of(db, dirs, agent_id).ok()?;
    let started_at = now_millis();
    let advanced = db
        .with_connection(|connection| store::advance_queue(connection, agent_id, started_at))
        .ok()??;
    Some(launch_of(
        registry,
        dirs,
        agent_id,
        executable,
        advanced.begun,
        advanced.text,
        started_at,
    ))
}

pub fn sweep_interrupted(db: &Database) -> Result<usize, String> {
    db.with_connection(|connection| store::sweep_interrupted(connection, now_millis()))
        .map_err(|error| format!("could not recover running turns: {error}"))
}

async fn drive_one(app: &AppHandle, launch: Launch) {
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
    runner::run(&db, app, &registry, launch, key).await;
}

async fn next_launch(app: &AppHandle, agent_id: String) -> Option<Launch> {
    let next_app = app.clone();
    let next = tauri::async_runtime::spawn_blocking(move || {
        let db = next_app.try_state::<Database>()?;
        let registry = next_app.try_state::<TurnRegistry>()?;
        start_next(&db, &registry, &providers::search_dirs(), &agent_id)
            .inspect(|launch| emit_queue_changed(&next_app, &launch.agent_id))
    })
    .await;
    next.ok().flatten()
}

async fn drive(app: AppHandle, first: Launch) {
    let mut current = Some(first);
    while let Some(launch) = current.take() {
        let agent_id = launch.agent_id.clone();
        drive_one(&app, launch).await;
        current = next_launch(&app, agent_id).await;
    }
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    agent_id: String,
    text: String,
) -> Result<SendResult, TurnError> {
    let prepare_app = app.clone();
    let queue_agent = agent_id.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let db = prepare_app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        let registry = prepare_app
            .try_state::<TurnRegistry>()
            .ok_or_else(TurnError::unavailable)?;
        prepare(&db, &registry, &providers::search_dirs(), &agent_id, &text)
    })
    .await
    .map_err(TurnError::database)??;
    match prepared {
        Prepared::Started(launch) => {
            let turn_id = launch.turn_id.clone();
            tauri::async_runtime::spawn(drive(app, launch));
            Ok(SendResult::Started { turn_id })
        }
        Prepared::Queued(queued) => {
            emit_queue_changed(&app, &queue_agent);
            Ok(SendResult::Queued {
                queued_id: queued.id,
            })
        }
    }
}

#[tauri::command]
pub async fn stop_turn(app: AppHandle, agent_id: String) {
    let Some(registry) = app.try_state::<TurnRegistry>() else {
        return;
    };
    if !registry.request_stop(&agent_id) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Some(registry) = app.try_state::<TurnRegistry>() {
            escalate_stop(&registry, &agent_id, EXIT_TERM_GRACE).await;
        }
    });
}

#[tauri::command]
pub async fn list_queue(app: AppHandle, agent_id: String) -> Result<Vec<QueuedMessage>, TurnError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        db.with_connection(|connection| store::list_queue(connection, &agent_id))
            .map_err(TurnError::database)
    })
    .await
    .map_err(TurnError::database)?
}

fn edit_queue_entry(
    db: &Database,
    queued_id: &str,
    text: &str,
) -> Result<(String, QueuedMessage), TurnError> {
    validate(text)?;
    db.with_connection(|connection| store::edit_queued(connection, queued_id, text))
        .map_err(TurnError::database)?
        .ok_or_else(|| TurnError::new("not-found", "This message already started or was removed."))
}

#[tauri::command]
pub async fn edit_queued(
    app: AppHandle,
    queued_id: String,
    text: String,
) -> Result<QueuedMessage, TurnError> {
    let edit_app = app.clone();
    let (agent_id, queued) = tauri::async_runtime::spawn_blocking(move || {
        let db = edit_app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        edit_queue_entry(&db, &queued_id, &text)
    })
    .await
    .map_err(TurnError::database)??;
    emit_queue_changed(&app, &agent_id);
    Ok(queued)
}

#[tauri::command]
pub async fn remove_queued(app: AppHandle, queued_id: String) -> Result<(), TurnError> {
    let remove_app = app.clone();
    let removed = tauri::async_runtime::spawn_blocking(move || {
        let db = remove_app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        db.with_connection(|connection| store::remove_queued(connection, &queued_id))
            .map_err(TurnError::database)
    })
    .await
    .map_err(TurnError::database)??;
    if let Some(agent_id) = removed {
        emit_queue_changed(&app, &agent_id);
    }
    Ok(())
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
        registry: TurnRegistry,
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
                registry: TurnRegistry::default(),
            }
        }

        fn prepare(&self, agent: &str, text: &str) -> Result<Prepared, TurnError> {
            prepare(
                &self.db,
                &self.registry,
                &[self.bin.path().to_path_buf()],
                agent,
                text,
            )
        }

        fn send(&self, agent: &str, text: &str) -> Result<Launch, TurnError> {
            match self.prepare(agent, text)? {
                Prepared::Started(launch) => Ok(launch),
                Prepared::Queued(_) => panic!("expected a started turn"),
            }
        }

        fn queue(&self, agent: &str, text: &str) -> QueuedMessage {
            match self.prepare(agent, text).unwrap() {
                Prepared::Queued(queued) => queued,
                Prepared::Started(_) => panic!("expected a queued message"),
            }
        }

        fn queued(&self) -> Vec<(String, i64)> {
            self.db
                .with_connection(|c| store::list_queue(c, "a"))
                .unwrap()
                .into_iter()
                .map(|queued| (queued.text, queued.position))
                .collect()
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

    fn prepared_code(result: Result<Prepared, TurnError>) -> &'static str {
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
    fn a_send_while_a_turn_runs_is_queued_with_increasing_positions() {
        let fx = Fixture::new("claude-code");
        fx.send("a", "one").unwrap();
        let first = fx.queue("a", "two");
        let second = fx.queue("a", "three");
        assert_eq!((first.position, second.position), (0, 1));
        assert_eq!(fx.queued(), [("two".into(), 0), ("three".into(), 1)]);
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn a_queued_message_is_validated_like_a_direct_one() {
        let fx = Fixture::new("claude-code");
        fx.send("a", "one").unwrap();
        assert_eq!(prepared_code(fx.prepare("a", "  ")), "empty");
        assert_eq!(
            prepared_code(fx.prepare("a", &"x".repeat(MESSAGE_MAX_CHARS + 1))),
            "too-long"
        );
        fx.queue("a", &"ก".repeat(MESSAGE_MAX_CHARS));
        assert_eq!(fx.queued().len(), 1);
    }

    #[test]
    fn a_queued_message_can_be_edited_and_removed_until_it_starts() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        let two = fx.queue("a", "two");
        let three = fx.queue("a", "three");
        assert_eq!(
            edit_queue_entry(&fx.db, &two.id, " ").unwrap_err().code,
            "empty"
        );
        assert_eq!(
            edit_queue_entry(&fx.db, &two.id, &"x".repeat(MESSAGE_MAX_CHARS + 1))
                .unwrap_err()
                .code,
            "too-long"
        );
        let (agent, edited) = edit_queue_entry(&fx.db, &two.id, "two, edited").unwrap();
        assert_eq!((agent.as_str(), edited.text.as_str()), ("a", "two, edited"));
        assert_eq!(edited.position, 0);
        let removed = fx
            .db
            .with_connection(|c| store::remove_queued(c, &three.id))
            .unwrap();
        assert_eq!(removed.as_deref(), Some("a"));
        assert_eq!(fx.queued(), [("two, edited".into(), 0)]);
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "completed", None, 9).unwrap()
        });
        let next = start_next(&fx.db, &fx.registry, &[fx.bin.path().to_path_buf()], "a").unwrap();
        assert_eq!(next.text, "two, edited");
        assert_eq!(
            edit_queue_entry(&fx.db, &two.id, "late").unwrap_err().code,
            "not-found"
        );
        let removed_twice = fx
            .db
            .with_connection(|c| store::remove_queued(c, &three.id))
            .unwrap();
        assert!(removed_twice.is_none());
    }

    #[test]
    fn the_queue_advances_one_message_at_a_time_after_any_end() {
        for status in ["completed", "failed", "stopped"] {
            let fx = Fixture::new("claude-code");
            let first = fx.send("a", "one").unwrap();
            fx.queue("a", "two");
            fx.queue("a", "three");
            let dirs = [fx.bin.path().to_path_buf()];
            assert!(start_next(&fx.db, &fx.registry, &dirs, "a").is_none());
            fx.db.with_connection(|c| {
                store::finish_turn(c, &first.turn_id, "a", status, None, 9).unwrap()
            });
            let second = start_next(&fx.db, &fx.registry, &dirs, "a").unwrap();
            assert_eq!(second.text, "two");
            assert_eq!(fx.queued(), [("three".into(), 1)]);
            assert_eq!(fx.turns(), 2);
            assert!(start_next(&fx.db, &fx.registry, &dirs, "a").is_none());
            assert_eq!(fx.queued(), [("three".into(), 1)]);
            assert_eq!(fx.turns(), 2);
        }
    }

    #[test]
    fn an_interrupted_turn_does_not_advance_the_queue_and_the_queue_survives_a_reopen() {
        let data = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let claude = bin.path().join("claude");
        fs::write(&claude, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = [bin.path().to_path_buf()];
        let registry = TurnRegistry::default();
        {
            let db = Database::open(data.path()).unwrap();
            db.with_connection(|c| {
                c.execute(
                    "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                     VALUES ('a', 'Ada', 'red', 'claude-code', 'haiku', '/w', 1, 1)",
                    [],
                )
                .unwrap();
            });
            prepare(&db, &registry, &dirs, "a", "one").unwrap();
            prepare(&db, &registry, &dirs, "a", "two").unwrap();
        }
        let db = Database::open(data.path()).unwrap();
        assert_eq!(sweep_interrupted(&db).unwrap(), 1);
        let queue = db.with_connection(|c| store::list_queue(c, "a")).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].text, "two");
        let turns: i64 = db
            .with_connection(|c| c.query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(turns, 1);
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
        assert!(registry.slots().is_empty());
    }

    #[test]
    fn a_stop_with_no_running_turn_is_a_no_op() {
        let registry = TurnRegistry::default();
        assert!(!registry.request_stop("a"));
        assert!(!registry.stop_requested("a"));
        assert!(registry.slots().is_empty());
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
