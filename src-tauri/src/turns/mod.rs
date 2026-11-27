mod claude_code;
mod redact;
mod runner;
mod store;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::agents::now_millis;
use crate::api_key;
use crate::auth;
use crate::db::Database;
use crate::providers::{self, CLAUDE_CODE_ID};
use crate::secrets::{SecretStore, SharedStore};
use store::{BeginError, ConversationPage, QueuedMessage, Sent, StepView};

pub const EVENT_NAME: &str = "turn-event";
pub const QUEUE_EVENT_NAME: &str = "queue-changed";
pub const MESSAGE_MAX_CHARS: usize = 100_000;
const EXIT_TERM_GRACE: Duration = Duration::from_secs(2);
const EXIT_POLL: Duration = Duration::from_millis(25);
pub const STALL_TIMEOUT: Duration = Duration::from_secs(300);
pub const TOOL_STALL_TIMEOUT: Duration = Duration::from_secs(900);
#[cfg(feature = "e2e")]
const STALL_SECONDS_ENV: &str = "SHIKIGAMI_STALL_SECONDS";

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

    fn shutting_down() -> Self {
        Self::new("shutting-down", "Shikigami is closing.")
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

struct Slot {
    turn_id: String,
    group: Option<i32>,
    stop_requested: bool,
}

struct StopTarget {
    group: Option<i32>,
}

#[derive(Default)]
pub struct TurnRegistry {
    slots: Mutex<HashMap<String, Slot>>,
    closing: AtomicBool,
}

impl TurnRegistry {
    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn is_closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    fn begin(&self, agent_id: &str, turn_id: &str) -> bool {
        let mut slots = self.slots();
        if self.is_closing() {
            return false;
        }
        slots.insert(
            agent_id.to_owned(),
            Slot {
                turn_id: turn_id.to_owned(),
                group: None,
                stop_requested: false,
            },
        );
        true
    }

    fn set_group(&self, agent_id: &str, turn_id: &str, group: i32) -> bool {
        let mut slots = self.slots();
        let slot = slots.entry(agent_id.to_owned()).or_insert_with(|| Slot {
            turn_id: turn_id.to_owned(),
            group: None,
            stop_requested: false,
        });
        if slot.turn_id != turn_id {
            return false;
        }
        slot.group = Some(group);
        if self.is_closing() {
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
            return false;
        }
        if slot.stop_requested {
            unsafe {
                libc::killpg(group, libc::SIGTERM);
            }
        }
        slot.stop_requested
    }

    fn clear_group(&self, agent_id: &str, turn_id: &str) {
        if let Some(slot) = self
            .slots()
            .get_mut(agent_id)
            .filter(|slot| slot.turn_id == turn_id)
        {
            slot.group = None;
        }
    }

    fn clear(&self, agent_id: &str, turn_id: &str) {
        let mut slots = self.slots();
        if slots
            .get(agent_id)
            .is_some_and(|slot| slot.turn_id == turn_id)
        {
            slots.remove(agent_id);
        }
    }

    fn stop_requested(&self, agent_id: &str, turn_id: &str) -> bool {
        self.slots()
            .get(agent_id)
            .is_some_and(|slot| slot.turn_id == turn_id && slot.stop_requested)
    }

    fn request_stop(&self, agent_id: &str, turn_id: &str) -> Option<StopTarget> {
        let mut slots = self.slots();
        let slot = slots
            .get_mut(agent_id)
            .filter(|slot| slot.turn_id == turn_id)?;
        slot.stop_requested = true;
        if let Some(group) = slot.group {
            unsafe {
                libc::killpg(group, libc::SIGTERM);
            }
        }
        Some(StopTarget { group: slot.group })
    }

    fn terminate(&self, agent_id: &str, turn_id: &str) -> Option<i32> {
        let group = self
            .slots()
            .get(agent_id)
            .filter(|slot| slot.turn_id == turn_id)
            .and_then(|slot| slot.group)?;
        unsafe {
            libc::killpg(group, libc::SIGTERM);
        }
        Some(group)
    }

    fn kill(&self, agent_id: &str, turn_id: &str) {
        let group = self
            .slots()
            .get(agent_id)
            .filter(|slot| slot.turn_id == turn_id)
            .and_then(|slot| slot.group);
        if let Some(group) = group {
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }

    pub fn kill_all(&self, grace: Duration) {
        self.closing.store(true, Ordering::SeqCst);
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

async fn kill_group_after(group: i32, grace: Duration) {
    tokio::time::sleep(grace).await;
    unsafe {
        libc::killpg(group, libc::SIGKILL);
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
    pub stall_timeout: Duration,
    pub tool_stall_timeout: Duration,
}

fn stall_timeout() -> Duration {
    #[cfg(feature = "e2e")]
    if let Some(seconds) = std::env::var(STALL_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        return Duration::from_secs(seconds);
    }
    STALL_TIMEOUT
}

fn begin_error(error: BeginError) -> TurnError {
    match error {
        BeginError::NotFound => TurnError::new("not-found", "This agent no longer exists."),
        BeginError::Busy => TurnError::new("busy", "This agent is already working."),
        BeginError::Database(error) => TurnError::database(error),
    }
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
) -> Option<Launch> {
    if !registry.begin(agent_id, &begun.turn_id) {
        return None;
    }
    Some(Launch {
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
        stall_timeout: stall_timeout(),
        tool_stall_timeout: TOOL_STALL_TIMEOUT,
    })
}

enum Prepared {
    Started(Launch),
    Queued {
        queued: QueuedMessage,
        launch: Option<Launch>,
    },
}

fn prepare(
    db: &Database,
    registry: &TurnRegistry,
    dirs: &[PathBuf],
    agent_id: &str,
    text: &str,
) -> Result<Prepared, TurnError> {
    if registry.is_closing() {
        return Err(TurnError::shutting_down());
    }
    validate(text)?;
    let executable = executable_of(db, dirs, agent_id)?;
    let started_at = now_millis();
    let sent = db
        .with_connection(|connection| store::begin_or_queue(connection, agent_id, text, started_at))
        .map_err(begin_error)?;
    match sent {
        Sent::Started(begun) => launch_of(
            registry,
            dirs,
            agent_id,
            executable,
            begun,
            text.to_owned(),
            started_at,
        )
        .map(Prepared::Started)
        .ok_or_else(TurnError::shutting_down),
        Sent::Queued { queued, started } => {
            let launch = started
                .map(|advanced| {
                    launch_of(
                        registry,
                        dirs,
                        agent_id,
                        executable,
                        advanced.begun,
                        advanced.text,
                        started_at,
                    )
                    .ok_or_else(TurnError::shutting_down)
                })
                .transpose()?;
            Ok(Prepared::Queued { queued, launch })
        }
    }
}

fn key_for_agent(db: &Database, store: &dyn SecretStore, agent_id: &str) -> runner::ApiKey {
    match auth::mode_for_agent(db, store, agent_id) {
        Ok(mode) => {
            runner::ApiKey::from_read(mode, api_key::key_for_turn(store, mode, CLAUDE_CODE_ID))
        }
        Err(error) if error.code == auth::AuthError::KEYCHAIN_FAILED => {
            runner::ApiKey::Unreadable(error.status)
        }
        Err(_) => runner::ApiKey::Unresolved,
    }
}

fn start_next(
    db: &Database,
    registry: &TurnRegistry,
    dirs: &[PathBuf],
    agent_id: &str,
) -> Option<Launch> {
    if registry.is_closing() {
        return None;
    }
    let executable = executable_of(db, dirs, agent_id).ok()?;
    let started_at = now_millis();
    let advanced = db
        .with_connection(|connection| {
            if registry.is_closing() {
                Ok(None)
            } else {
                store::advance_queue(connection, agent_id, started_at)
            }
        })
        .ok()??;
    launch_of(
        registry,
        dirs,
        agent_id,
        executable,
        advanced.begun,
        advanced.text,
        started_at,
    )
}

fn resume(
    db: &Database,
    registry: &TurnRegistry,
    dirs: &[PathBuf],
    agent_id: &str,
) -> Result<Option<Launch>, TurnError> {
    if registry.is_closing() {
        return Err(TurnError::shutting_down());
    }
    let executable = executable_of(db, dirs, agent_id)?;
    let started_at = now_millis();
    let advanced = db
        .with_connection(|connection| {
            if registry.is_closing() {
                Ok(None)
            } else {
                store::advance_queue(connection, agent_id, started_at)
            }
        })
        .or_else(|error| match error {
            BeginError::Busy => Ok(None),
            other => Err(begin_error(other)),
        })?;
    advanced
        .map(|advanced| {
            launch_of(
                registry,
                dirs,
                agent_id,
                executable,
                advanced.begun,
                advanced.text,
                started_at,
            )
            .ok_or_else(TurnError::shutting_down)
        })
        .transpose()
}

pub fn sweep_interrupted(db: &Database) -> Result<usize, String> {
    db.with_connection(|connection| store::sweep_interrupted(connection, now_millis()))
        .map_err(|error| format!("could not recover running turns: {error}"))
}

async fn drive_one(app: &AppHandle, launch: Launch) -> bool {
    let key_app = app.clone();
    let key_agent = launch.agent_id.clone();
    let key = tauri::async_runtime::spawn_blocking(move || {
        let (Some(db), Some(store)) = (
            key_app.try_state::<Database>(),
            key_app.try_state::<SharedStore>(),
        ) else {
            return runner::ApiKey::Unresolved;
        };
        key_for_agent(&db, &**store, &key_agent)
    })
    .await
    .unwrap_or(runner::ApiKey::Unreadable(None));
    let (Some(db), Some(registry)) = (app.try_state::<Database>(), app.try_state::<TurnRegistry>())
    else {
        return false;
    };
    runner::run(&db, app, &registry, launch, key).await
}

async fn next_launch(app: &AppHandle, agent_id: String) -> Option<Launch> {
    let next_app = app.clone();
    let next = tauri::async_runtime::spawn_blocking(move || {
        let db = next_app.try_state::<Database>()?;
        let registry = next_app.try_state::<TurnRegistry>()?;
        let launch = start_next(&db, &registry, &providers::search_dirs(), &agent_id);
        emit_queue_changed(&next_app, &agent_id);
        launch
    })
    .await;
    next.ok().flatten()
}

async fn drive(app: AppHandle, first: Launch) {
    let mut current = Some(first);
    while let Some(launch) = current.take() {
        let agent_id = launch.agent_id.clone();
        let advance = drive_one(&app, launch).await;
        let closing = app
            .try_state::<TurnRegistry>()
            .is_none_or(|registry| registry.is_closing());
        if advance && !closing {
            current = next_launch(&app, agent_id).await;
        }
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
        Prepared::Queued { queued, launch } => {
            emit_queue_changed(&app, &queue_agent);
            if let Some(launch) = launch {
                tauri::async_runtime::spawn(drive(app, launch));
            }
            Ok(SendResult::Queued {
                queued_id: queued.id,
            })
        }
    }
}

#[tauri::command]
pub async fn resume_queue(app: AppHandle, agent_id: String) -> Result<(), TurnError> {
    let resume_app = app.clone();
    let queue_agent = agent_id.clone();
    let launch = tauri::async_runtime::spawn_blocking(move || {
        let db = resume_app
            .try_state::<Database>()
            .ok_or_else(TurnError::unavailable)?;
        let registry = resume_app
            .try_state::<TurnRegistry>()
            .ok_or_else(TurnError::unavailable)?;
        resume(&db, &registry, &providers::search_dirs(), &agent_id)
    })
    .await
    .map_err(TurnError::database)??;
    if let Some(launch) = launch {
        emit_queue_changed(&app, &queue_agent);
        tauri::async_runtime::spawn(drive(app, launch));
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_turn(app: AppHandle, agent_id: String, turn_id: String) {
    let Some(registry) = app.try_state::<TurnRegistry>() else {
        return;
    };
    let Some(target) = registry.request_stop(&agent_id, &turn_id) else {
        return;
    };
    if let Some(group) = target.group {
        tauri::async_runtime::spawn(kill_group_after(group, EXIT_TERM_GRACE));
    }
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
    use crate::auth::AuthMode;
    use crate::secrets::{MemoryStore, SecretError, SecretString};
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
                Prepared::Queued { .. } => panic!("expected a started turn"),
            }
        }

        fn queue(&self, agent: &str, text: &str) -> QueuedMessage {
            match self.prepare(agent, text).unwrap() {
                Prepared::Queued { queued, .. } => queued,
                Prepared::Started(_) => panic!("expected a queued message"),
            }
        }

        fn queue_with_launch(&self, agent: &str, text: &str) -> (QueuedMessage, Option<Launch>) {
            match self.prepare(agent, text).unwrap() {
                Prepared::Queued { queued, launch } => (queued, launch),
                Prepared::Started(_) => panic!("expected a queued message"),
            }
        }

        fn dirs(&self) -> [PathBuf; 1] {
            [self.bin.path().to_path_buf()]
        }

        fn queued(&self) -> Vec<(String, i64)> {
            self.db
                .with_connection(|c| store::list_queue(c, "a"))
                .unwrap()
                .into_iter()
                .map(|queued| (queued.text, queued.position))
                .collect()
        }

        fn queued_id(&self) -> String {
            self.db
                .with_connection(|c| store::list_queue(c, "a"))
                .unwrap()
                .remove(0)
                .id
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
        assert!(registry.begin("a", "t1"));
        registry.set_group("a", "t1", group_of(&first));
        assert!(registry.begin("b", "t2"));
        registry.set_group("b", "t2", group_of(&second));
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
    fn a_group_that_registers_after_the_exit_kill_is_killed_and_not_advanced() {
        use std::os::unix::process::ExitStatusExt;
        let registry = TurnRegistry::default();
        registry.kill_all(Duration::from_millis(10));
        assert!(!registry.begin("a", "t1"));
        let mut late = spawn_group("sleep 30");
        let group = group_of(&late);
        let reaper = std::thread::spawn(move || late.wait().unwrap());
        assert!(!registry.set_group("a", "t1", group));
        let status = reaper.join().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn a_stop_with_no_running_turn_is_a_no_op() {
        let registry = TurnRegistry::default();
        assert!(registry.request_stop("a", "t").is_none());
        assert!(!registry.stop_requested("a", "t"));
        assert!(registry.slots().is_empty());
    }

    #[test]
    fn a_stop_names_its_turn_so_a_stale_request_cannot_stop_the_next_turn() {
        let registry = TurnRegistry::default();
        assert!(registry.begin("a", "t2"));
        assert!(registry.request_stop("a", "t1").is_none());
        assert!(!registry.stop_requested("a", "t2"));
        assert!(registry.request_stop("a", "t2").is_some());
        assert!(registry.stop_requested("a", "t2"));
        assert!(!registry.stop_requested("a", "t1"));
        registry.clear("a", "t1");
        assert!(registry.stop_requested("a", "t2"));
        registry.clear("a", "t2");
        assert!(registry.slots().is_empty());
    }

    #[test]
    fn an_earlier_turns_kill_timer_cannot_kill_a_later_turns_process() {
        let registry = TurnRegistry::default();
        let mut old = spawn_group("sleep 30");
        assert!(registry.begin("a", "t1"));
        registry.set_group("a", "t1", group_of(&old));
        let target = registry.request_stop("a", "t1").unwrap();
        old.wait().unwrap();
        registry.clear("a", "t1");
        let mut next = spawn_group("sleep 30");
        assert!(registry.begin("a", "t2"));
        registry.set_group("a", "t2", group_of(&next));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(kill_group_after(
            target.group.unwrap(),
            Duration::from_millis(50),
        ));
        assert!(next.try_wait().unwrap().is_none());
        assert!(!registry.stop_requested("a", "t2"));
        registry.kill("a", "t2");
        next.wait().unwrap();
    }

    #[test]
    fn a_stop_before_the_group_registers_terminates_it_when_it_registers() {
        use std::os::unix::process::ExitStatusExt;
        let registry = TurnRegistry::default();
        assert!(registry.begin("a", "t"));
        let target = registry.request_stop("a", "t").unwrap();
        assert!(target.group.is_none());
        let mut child = spawn_group("sleep 30");
        assert!(registry.set_group("a", "t", group_of(&child)));
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn the_kill_after_the_grace_reaches_a_group_whose_leader_already_exited() {
        let registry = TurnRegistry::default();
        let mut leader = spawn_group("(trap '' TERM; exec sleep 30) & exec sleep 30");
        let group = group_of(&leader);
        assert!(registry.begin("a", "t"));
        registry.set_group("a", "t", group);
        std::thread::sleep(Duration::from_millis(300));
        let target = registry.request_stop("a", "t").unwrap();
        leader.wait().unwrap();
        registry.clear_group("a", "t");
        assert_eq!(unsafe { libc::killpg(group, 0) }, 0);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(kill_group_after(
            target.group.unwrap(),
            Duration::from_millis(200),
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::killpg(group, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(EXIT_POLL);
        }
        assert_ne!(unsafe { libc::killpg(group, 0) }, 0);
    }

    #[test]
    fn shutdown_with_a_running_turn_and_two_queued_messages_starts_nothing_more() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        fx.queue("a", "three");
        let child = spawn_group("sleep 30");
        fx.registry.set_group("a", &first.turn_id, group_of(&child));
        let reaper = std::thread::spawn(move || {
            let mut child = child;
            child.wait().unwrap()
        });
        fx.registry.kill_all(Duration::from_secs(2));
        reaper.join().unwrap();
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "failed", None, 9).unwrap()
        });
        assert!(start_next(&fx.db, &fx.registry, &fx.dirs(), "a").is_none());
        assert_eq!(fx.queued(), [("two".into(), 0), ("three".into(), 1)]);
        assert_eq!(fx.turns(), 1);
        assert_eq!(prepared_code(fx.prepare("a", "four")), "shutting-down");
        assert_eq!(fx.queued().len(), 2);
        assert_eq!(fx.turns(), 1);
        assert!(!fx.registry.begin("a", "t9"));
    }

    #[test]
    fn a_new_message_never_overtakes_a_stalled_queue() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "failed", None, 9).unwrap()
        });
        let (queued, launch) = fx.queue_with_launch("a", "three");
        assert_eq!(queued.position, 1);
        assert_eq!(launch.unwrap().text, "two");
        assert_eq!(fx.queued(), [("three".into(), 1)]);
        assert_eq!(fx.turns(), 2);
    }

    #[test]
    fn a_queue_that_cannot_start_stays_untouched_and_the_next_send_still_queues() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "failed", None, 9).unwrap()
        });
        fs::remove_file(fx.bin.path().join("claude")).unwrap();
        assert!(start_next(&fx.db, &fx.registry, &fx.dirs(), "a").is_none());
        assert_eq!(fx.queued(), [("two".into(), 0)]);
        assert_eq!(
            prepared_code(fx.prepare("a", "three")),
            "provider-unavailable"
        );
        assert_eq!(fx.queued(), [("two".into(), 0)]);
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn exit_kills_a_group_that_ignores_the_term_signal_after_the_grace() {
        use std::os::unix::process::ExitStatusExt;
        let registry = TurnRegistry::default();
        let child = spawn_group("trap '' TERM; while true; do sleep 1; done");
        assert!(registry.begin("a", "t"));
        registry.set_group("a", "t", group_of(&child));
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

    #[test]
    fn resume_starts_the_head_of_a_queue_that_waits_after_an_interrupted_turn() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        fx.queue("a", "three");
        fx.db
            .with_connection(|c| store::sweep_interrupted(c, 9))
            .unwrap();
        let _ = first;
        let launch = resume(&fx.db, &fx.registry, &fx.dirs(), "a")
            .unwrap()
            .expect("the head of the queue starts");
        assert_eq!(launch.text, "two");
        assert_eq!(fx.queued(), [("three".into(), 1)]);
        assert_eq!(fx.turns(), 2);
    }

    #[test]
    fn resume_does_nothing_while_a_turn_runs_or_when_the_queue_is_empty() {
        let fx = Fixture::new("claude-code");
        assert!(resume(&fx.db, &fx.registry, &fx.dirs(), "a")
            .unwrap()
            .is_none());
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        assert!(resume(&fx.db, &fx.registry, &fx.dirs(), "a")
            .unwrap()
            .is_none());
        assert_eq!(fx.queued(), [("two".into(), 0)]);
        assert_eq!(fx.turns(), 1);
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "completed", None, 9).unwrap()
        });
        let queued_id = fx.queued_id();
        fx.db
            .with_connection(|c| store::remove_queued(c, &queued_id))
            .unwrap();
        assert!(resume(&fx.db, &fx.registry, &fx.dirs(), "a")
            .unwrap()
            .is_none());
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn resume_reports_a_missing_provider_and_a_failing_database() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "failed", None, 9).unwrap()
        });
        let error = resume(&fx.db, &fx.registry, &[], "a")
            .err()
            .expect("no provider");
        assert_eq!(error.code, "provider-unavailable");
        let error = resume(&fx.db, &fx.registry, &fx.dirs(), "nope")
            .err()
            .expect("no agent");
        assert_eq!(error.code, "not-found");
        assert_eq!(fx.queued(), [("two".into(), 0)]);
        fx.db
            .with_connection(|c| c.execute("DROP TABLE queued_messages", []).unwrap());
        let error = resume(&fx.db, &fx.registry, &fx.dirs(), "a")
            .err()
            .expect("database failure");
        assert_eq!(error.code, "database-failed");
        assert_eq!(fx.turns(), 1);
    }

    #[test]
    fn resume_is_refused_while_closing_and_leaves_the_queue_alone() {
        let fx = Fixture::new("claude-code");
        let first = fx.send("a", "one").unwrap();
        fx.queue("a", "two");
        fx.db.with_connection(|c| {
            store::finish_turn(c, &first.turn_id, "a", "failed", None, 9).unwrap()
        });
        fx.registry.kill_all(Duration::from_millis(10));
        let error = resume(&fx.db, &fx.registry, &fx.dirs(), "a")
            .err()
            .expect("refused");
        assert_eq!(error.code, "shutting-down");
        assert_eq!(fx.queued(), [("two".into(), 0)]);
        assert_eq!(fx.turns(), 1);
    }

    struct NeverTouched;

    impl SecretStore for NeverTouched {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            panic!("the Keychain was touched");
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            panic!("the Keychain was touched");
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            panic!("the Keychain was touched");
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            panic!("the Keychain was touched");
        }
    }

    fn store_with_key() -> MemoryStore {
        let store = MemoryStore::default();
        store
            .set("anthropic-api-key", &SecretString::new("sk-ant-saved-0000"))
            .unwrap();
        store
    }

    fn choose(fx: &Fixture, app: Option<AuthMode>, agent: Option<AuthMode>) {
        if let Some(mode) = app {
            auth::set_app_mode(&fx.db, mode).unwrap();
        }
        fx.db.with_connection(|c| {
            c.execute(
                "UPDATE agents SET auth_mode = ?1 WHERE id = 'a'",
                [agent.map(AuthMode::as_str)],
            )
            .unwrap()
        });
    }

    #[test]
    fn a_subscription_turn_never_touches_the_keychain() {
        let fx = Fixture::new("claude-code");
        choose(&fx, Some(AuthMode::Subscription), None);
        assert!(matches!(
            key_for_agent(&fx.db, &NeverTouched, "a"),
            runner::ApiKey::Subscription
        ));
        choose(&fx, Some(AuthMode::ApiKey), Some(AuthMode::Subscription));
        assert!(matches!(
            key_for_agent(&fx.db, &NeverTouched, "a"),
            runner::ApiKey::Subscription
        ));
    }

    #[test]
    fn an_api_key_turn_gets_the_saved_key_or_is_missing_one() {
        let fx = Fixture::new("claude-code");
        choose(&fx, Some(AuthMode::ApiKey), None);
        match key_for_agent(&fx.db, &store_with_key(), "a") {
            runner::ApiKey::Present(key) => assert_eq!(key.expose(), "sk-ant-saved-0000"),
            _ => panic!("expected the saved key"),
        }
        assert!(matches!(
            key_for_agent(&fx.db, &MemoryStore::default(), "a"),
            runner::ApiKey::Missing
        ));
    }

    #[test]
    fn an_agent_override_beats_the_app_default_in_both_directions() {
        let fx = Fixture::new("claude-code");
        choose(&fx, Some(AuthMode::Subscription), Some(AuthMode::ApiKey));
        assert!(matches!(
            key_for_agent(&fx.db, &store_with_key(), "a"),
            runner::ApiKey::Present(_)
        ));
        assert!(matches!(
            key_for_agent(&fx.db, &MemoryStore::default(), "a"),
            runner::ApiKey::Missing
        ));
        choose(&fx, Some(AuthMode::ApiKey), Some(AuthMode::Subscription));
        assert!(matches!(
            key_for_agent(&fx.db, &store_with_key(), "a"),
            runner::ApiKey::Subscription
        ));
    }

    struct BrokenStore;

    impl SecretStore for BrokenStore {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            Err(SecretError::Read(None))
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            Err(SecretError::Read(None))
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            Err(SecretError::Write)
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Delete)
        }
    }

    struct DeniedStore;

    impl SecretStore for DeniedStore {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            Err(SecretError::Read(Some(-25293)))
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            Err(SecretError::Read(Some(-25293)))
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            Err(SecretError::Write)
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Delete)
        }
    }

    #[test]
    fn the_keychain_status_code_reaches_the_api_key_of_a_turn() {
        let fx = Fixture::new("claude-code");
        assert!(matches!(
            key_for_agent(&fx.db, &DeniedStore, "a"),
            runner::ApiKey::Unreadable(Some(-25293))
        ));
        choose(&fx, Some(AuthMode::ApiKey), None);
        assert!(matches!(
            key_for_agent(&fx.db, &DeniedStore, "a"),
            runner::ApiKey::Unreadable(Some(-25293))
        ));
    }

    #[test]
    fn an_unreadable_keychain_with_nothing_chosen_fails_the_turn_instead_of_using_the_subscription()
    {
        let fx = Fixture::new("claude-code");
        assert!(matches!(
            key_for_agent(&fx.db, &BrokenStore, "a"),
            runner::ApiKey::Unreadable(None)
        ));
        choose(&fx, Some(AuthMode::Subscription), None);
        assert!(matches!(
            key_for_agent(&fx.db, &BrokenStore, "a"),
            runner::ApiKey::Subscription
        ));
    }

    #[test]
    fn with_nothing_chosen_the_key_decides() {
        let fx = Fixture::new("claude-code");
        assert!(matches!(
            key_for_agent(&fx.db, &MemoryStore::default(), "a"),
            runner::ApiKey::Subscription
        ));
        assert!(matches!(
            key_for_agent(&fx.db, &store_with_key(), "a"),
            runner::ApiKey::Present(_)
        ));
    }
}
