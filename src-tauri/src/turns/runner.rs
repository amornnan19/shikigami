use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::claude_code::{self, Parsed, Session, StepStart, StreamParser};
use super::group;
use super::redact::Redactor;
use super::store::{self, NewMessage, StepView};
use super::{
    kill_group_after, EventSink, Launch, TurnEnvelope, TurnEvent, TurnRegistry, EXIT_TERM_GRACE,
};
use crate::agents::now_millis;
use crate::api_key::ApiKeyError;
use crate::auth::AuthMode;
use crate::db::Database;
use crate::providers;
use crate::secrets::SecretString;

const LINE_LIMIT: usize = 1024 * 1024;
const STDERR_KEEP: usize = 64 * 1024;
const STDERR_LINE_LIMIT: usize = 4096;
const TEXT_WRITE_INTERVAL: Duration = Duration::from_millis(500);
const EXIT_GRACE: Duration = Duration::from_secs(5);
const STDERR_GRACE: Duration = Duration::from_secs(2);
const ALLOWED_VARIABLES: [&str; 7] = ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "TERM"];
const ALLOWED_PREFIX: &str = "LC_";
const SNAPSHOT_FLAG: &str = "--system-prompt-snapshot";
const SESSION_MISSING: &str = "No conversation found";

const RESUME_NOTICE: &str =
    "Started a new Claude Code session. The agent may have lost earlier context.";

const NO_KEY_SAVED: &str =
    "No API key is saved. Add one or switch this agent to your Claude subscription.";

pub enum ApiKey {
    Present(SecretString),
    Subscription,
    Missing,
    Unreadable(Option<i32>),
    Unresolved,
}

impl ApiKey {
    pub fn from_read(mode: AuthMode, read: Result<Option<SecretString>, ApiKeyError>) -> Self {
        match (mode, read) {
            (_, Err(error)) => Self::Unreadable(error.status),
            (_, Ok(Some(key))) => Self::Present(key),
            (AuthMode::Subscription, Ok(None)) => Self::Subscription,
            (AuthMode::ApiKey, Ok(None)) => Self::Missing,
        }
    }

    fn redactor(&self) -> Redactor {
        match self {
            Self::Present(key) => Redactor::for_key(Some(key)),
            Self::Subscription | Self::Missing | Self::Unreadable(_) | Self::Unresolved => {
                Redactor::none()
            }
        }
    }

    fn refusal(&self) -> Option<String> {
        match self {
            Self::Present(_) | Self::Subscription => None,
            Self::Missing => Some(NO_KEY_SAVED.to_owned()),
            Self::Unreadable(Some(status)) => Some(format!(
                "Could not read the saved API key (Keychain error {status})."
            )),
            Self::Unreadable(None) => Some("Could not read the saved API key.".to_owned()),
            Self::Unresolved => Some("Could not read the sign-in setting.".to_owned()),
        }
    }
}

fn is_allowed_variable(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| ALLOWED_VARIABLES.contains(&name) || name.starts_with(ALLOWED_PREFIX))
}

enum Line {
    Text(String),
    TooLong,
    End,
}

async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> io::Result<Line> {
    buffer.clear();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buffer.is_empty() {
                Line::End
            } else {
                Line::Text(String::from_utf8_lossy(buffer).into_owned())
            });
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if buffer.len() + take > LINE_LIMIT {
            return Ok(Line::TooLong);
        }
        buffer.extend_from_slice(&available[..take]);
        reader.consume(take + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Line::Text(String::from_utf8_lossy(buffer).into_owned()));
        }
    }
}

#[derive(Default)]
struct StderrSummary {
    last_line: Option<String>,
    session_missing: bool,
}

async fn read_stderr<R: AsyncReadExt + Unpin>(mut stderr: R) -> StderrSummary {
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut trimmed = false;
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                kept.extend_from_slice(&chunk[..count]);
                if kept.len() > STDERR_KEEP {
                    kept.drain(..kept.len() - STDERR_KEEP);
                    trimmed = true;
                }
            }
        }
    }
    if trimmed {
        let first_line_end = kept.iter().position(|byte| *byte == b'\n');
        kept.drain(..first_line_end.map_or(kept.len(), |end| end + 1));
    }
    let text = String::from_utf8_lossy(&kept);
    StderrSummary {
        last_line: text
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(|line| claude_code::truncate(line, STDERR_LINE_LIMIT)),
        session_missing: text.contains(SESSION_MISSING),
    }
}

struct TextRow {
    raw: String,
    emitted_raw: usize,
    shown: String,
    written: Instant,
    dirty: bool,
}

#[derive(Default)]
struct Attempt {
    spawned: bool,
    saw_init: bool,
    result: Option<(bool, String)>,
    failure: Option<String>,
    stderr: StderrSummary,
}

impl Attempt {
    fn succeeded(&self) -> bool {
        matches!(self.result, Some((false, _))) && self.failure.is_none()
    }

    fn session_missing(&self) -> bool {
        self.stderr.session_missing
            || self
                .result
                .as_ref()
                .is_some_and(|(_, text)| text.contains(SESSION_MISSING))
    }
}

struct Runner<'a> {
    db: &'a Database,
    sink: &'a dyn EventSink,
    registry: &'a TurnRegistry,
    redactor: Redactor,
    agent_id: String,
    group_id: Option<String>,
    turn_id: String,
    clock: i64,
    next_position: i64,
    texts: HashMap<String, TextRow>,
    step_inputs: HashMap<String, String>,
    stored_session: String,
    save_failed: bool,
    input_unread: bool,
    previous_seen: i64,
}

struct InstructionsFile(Option<PathBuf>);

impl InstructionsFile {
    fn write(temp_dir: &Path, text: &str) -> io::Result<Self> {
        if text.trim().is_empty() {
            return Ok(Self(None));
        }
        fs::create_dir_all(temp_dir)?;
        let path = temp_dir.join(format!("instructions-{}.md", uuid::Uuid::new_v4().simple()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let written = file.write_all(text.as_bytes());
        let guard = Self(Some(path));
        written?;
        Ok(guard)
    }

    fn path(&self) -> Option<&Path> {
        self.0.as_deref()
    }
}

impl Drop for InstructionsFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

fn safe_end(redactor: &Redactor, raw: &str) -> usize {
    let mut end = raw.len().saturating_sub(redactor.hold_back());
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    for (start, stop) in redactor.spans(raw) {
        if start < end && end < stop {
            end = start;
        }
    }
    end
}

impl Runner<'_> {
    fn tick(&mut self) -> i64 {
        self.clock = now_millis().max(self.clock + 1);
        self.clock
    }

    fn emit(&self, event: TurnEvent) {
        self.sink.emit(&TurnEnvelope {
            agent_id: self.agent_id.clone(),
            group_id: self.group_id.clone(),
            turn_id: self.turn_id.clone(),
            event,
        });
    }

    fn save(&mut self, write: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<()>) {
        if self.db.with_connection(write).is_err() {
            self.save_failed = true;
        }
    }

    fn add_notice(&mut self, text: &str) {
        let text = self.redactor.redact(text);
        let id = uuid::Uuid::now_v7().to_string();
        let created_at = self.tick();
        let (agent_id, turn_id, group_id) = (
            self.agent_id.clone(),
            self.turn_id.clone(),
            self.group_id.clone(),
        );
        self.save(|connection| {
            store::insert_message_in(
                connection,
                &NewMessage {
                    id: &id,
                    agent_id: &agent_id,
                    turn_id: &turn_id,
                    group_id: group_id.as_deref(),
                    role: "notice",
                    content: &text,
                    created_at,
                },
            )
        });
        self.emit(TurnEvent::Notice { item_id: id, text });
    }

    fn write_text(&mut self, item_id: &str) {
        let Some(row) = self.texts.get_mut(item_id) else {
            return;
        };
        row.written = Instant::now();
        row.dirty = false;
        let text = row.shown.clone();
        self.save(|connection| store::update_message(connection, item_id, &text));
    }

    fn ensure_text_row(&mut self, item_id: &str) {
        if self.texts.contains_key(item_id) {
            return;
        }
        let created_at = self.tick();
        let (agent_id, turn_id, group_id) = (
            self.agent_id.clone(),
            self.turn_id.clone(),
            self.group_id.clone(),
        );
        self.save(|connection| {
            store::insert_message_in(
                connection,
                &NewMessage {
                    id: item_id,
                    agent_id: &agent_id,
                    turn_id: &turn_id,
                    group_id: group_id.as_deref(),
                    role: "assistant",
                    content: "",
                    created_at,
                },
            )
        });
        self.texts.insert(
            item_id.to_owned(),
            TextRow {
                raw: String::new(),
                emitted_raw: 0,
                shown: String::new(),
                written: Instant::now(),
                dirty: false,
            },
        );
    }

    fn stream_text(&mut self, item_id: String, text: &str) {
        self.ensure_text_row(&item_id);
        let Some(row) = self.texts.get_mut(&item_id) else {
            return;
        };
        row.raw.push_str(text);
        let end = safe_end(&self.redactor, &row.raw).max(row.emitted_raw);
        if end == row.emitted_raw {
            return;
        }
        let piece = self.redactor.redact(&row.raw[row.emitted_raw..end]);
        row.emitted_raw = end;
        row.shown.push_str(&piece);
        row.dirty = true;
        let due = row.written.elapsed() >= TEXT_WRITE_INTERVAL;
        if due {
            self.write_text(&item_id);
        }
        self.emit(TurnEvent::TextDelta {
            item_id,
            text: piece,
        });
    }

    fn set_text(&mut self, item_id: String, text: &str) {
        self.ensure_text_row(&item_id);
        let shown = self.redactor.redact(text);
        if let Some(row) = self.texts.get_mut(&item_id) {
            row.raw = text.to_owned();
            row.emitted_raw = row.raw.len();
            row.shown.clone_from(&shown);
        }
        self.write_text(&item_id);
        self.emit(TurnEvent::TextSet {
            item_id,
            text: shown,
        });
    }

    fn flush_texts(&mut self) {
        let pending: Vec<String> = self
            .texts
            .iter()
            .filter(|(_, row)| row.emitted_raw < row.raw.len() || row.dirty)
            .map(|(id, _)| id.clone())
            .collect();
        for id in pending {
            let Some(row) = self.texts.get_mut(&id) else {
                continue;
            };
            let rest = self.redactor.redact(&row.raw[row.emitted_raw..]);
            row.emitted_raw = row.raw.len();
            row.shown.push_str(&rest);
            self.write_text(&id);
            if !rest.is_empty() {
                self.emit(TurnEvent::TextDelta {
                    item_id: id,
                    text: rest,
                });
            }
        }
    }

    fn start_step(&mut self, step: StepStart) {
        let view = StepView {
            id: step.id.clone(),
            kind: step.kind,
            title: step.title,
            detail: Some(step.input.clone()),
            status: "running".to_owned(),
        };
        let position = self.next_position;
        self.next_position += 1;
        let created_at = self.tick();
        let turn_id = self.turn_id.clone();
        self.save(|connection| {
            store::insert_step(connection, &turn_id, position, &view, created_at)
        });
        self.step_inputs.insert(step.id, step.input);
        self.emit(TurnEvent::StepStarted { step: view });
    }

    fn end_step(&mut self, step_id: String, failed: bool, output: String) {
        let input = self.step_inputs.remove(&step_id).unwrap_or_default();
        let detail = if output.is_empty() {
            input
        } else {
            format!("{input}\n\n{output}")
        };
        let status = if failed { "failed" } else { "done" };
        self.save(|connection| store::end_step(connection, &step_id, status, Some(&detail)));
        self.emit(TurnEvent::StepEnded {
            step_id,
            status: status.to_owned(),
            detail: Some(detail),
        });
    }

    fn handle(&mut self, parsed: Parsed, attempt: &mut Attempt) {
        match parsed {
            Parsed::Session(id) => {
                attempt.saw_init = true;
                if id != self.stored_session {
                    self.store_session(Some(&id));
                    self.stored_session = id;
                }
            }
            Parsed::TextDelta { item_id, text } => self.stream_text(item_id, &text),
            Parsed::TextSet { item_id, text } => self.set_text(item_id, &text),
            Parsed::StepStarted(step) => self.start_step(step),
            Parsed::StepEnded {
                step_id,
                failed,
                output,
            } => self.end_step(step_id, failed, output),
            Parsed::Result { is_error, text } => attempt.result = Some((is_error, text)),
        }
    }

    fn store_session(&mut self, session_id: Option<&str>) {
        let agent_id = self.agent_id.clone();
        match self.group_id.clone() {
            Some(group_id) => self.save(|connection| {
                group::set_session(connection, &agent_id, &group_id, session_id)
            }),
            None => self.save(|connection| store::set_session(connection, &agent_id, session_id)),
        }
    }

    fn finish(&mut self, status: &str, error: Option<String>) {
        self.flush_texts();
        let error = error.map(|message| self.redactor.redact(&message));
        let (status, error) = if status == "completed" && self.save_failed {
            ("failed", Some("Could not save the whole reply.".to_owned()))
        } else {
            (status, error)
        };
        let now = self.tick();
        let (turn_id, agent_id) = (self.turn_id.clone(), self.agent_id.clone());
        let group_id = self.group_id.clone();
        let restore_seen = self.input_unread.then_some(self.previous_seen);
        self.save(|connection| match &group_id {
            Some(group_id) => group::finish_turn(
                connection,
                &group::Finish {
                    turn_id: &turn_id,
                    agent_id: &agent_id,
                    group_id,
                    status,
                    error: error.as_deref(),
                    restore_seen,
                    now,
                },
            ),
            None => store::finish_turn(
                connection,
                &turn_id,
                &agent_id,
                status,
                error.as_deref(),
                now,
            ),
        });
        self.emit(TurnEvent::Ended {
            status: status.to_owned(),
            error,
        });
    }

    fn command(
        &self,
        launch: &Launch,
        session: &Session,
        key: &ApiKey,
        group_flags: Option<&claude_code::GroupFlags>,
    ) -> Option<Command> {
        let search_path = env::join_paths(&launch.search_dirs).ok()?;
        let mut command = Command::new(&launch.executable);
        command
            .args(claude_code::arguments(&launch.model, session, group_flags))
            .current_dir(&launch.workspace)
            .env_clear()
            .envs(
                launch
                    .environment
                    .iter()
                    .filter(|(name, _)| is_allowed_variable(name))
                    .cloned(),
            )
            .env("PATH", search_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if let ApiKey::Present(secret) = key {
            command.env("ANTHROPIC_API_KEY", secret.expose());
        }
        Some(command)
    }

    async fn attempt(&mut self, launch: &Launch, session: &Session, key: &ApiKey) -> Attempt {
        let mut attempt = Attempt::default();
        if self.registry.is_closing() || self.registry.stop_requested(&self.agent_id, &self.turn_id)
        {
            return attempt;
        }
        let instructions = match &launch.group {
            Some(group) => match InstructionsFile::write(&group.temp_dir, &group.instructions) {
                Ok(file) => Some(file),
                Err(_) => {
                    attempt.failure = Some("Could not start Claude Code.".to_owned());
                    return attempt;
                }
            },
            None => None,
        };
        let snapshot_off = match &launch.group {
            Some(_) => {
                providers::help_lists_flag(&launch.executable, &launch.search_dirs, SNAPSHOT_FLAG)
                    .await
            }
            None => false,
        };
        let group_flags = instructions
            .as_ref()
            .map(|file| claude_code::GroupFlags {
                instructions_file: file.path(),
                snapshot_off,
            })
            .or_else(|| {
                launch.group.as_ref().map(|_| claude_code::GroupFlags {
                    instructions_file: None,
                    snapshot_off,
                })
            });
        let Some(mut command) = self.command(launch, session, key, group_flags.as_ref()) else {
            attempt.failure = Some("Could not start Claude Code.".to_owned());
            return attempt;
        };
        let Ok(mut child) = command.spawn() else {
            attempt.failure = Some("Could not start Claude Code.".to_owned());
            return attempt;
        };
        attempt.spawned = true;
        if let Some(group) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
            if self
                .registry
                .set_group(&self.agent_id, &self.turn_id, group)
            {
                tokio::spawn(kill_group_after(group, EXIT_TERM_GRACE));
            }
        }
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            attempt.failure = Some("Could not start Claude Code.".to_owned());
            self.registry.kill(&self.agent_id, &self.turn_id);
            let _ = child.wait().await;
            self.registry.clear_group(&self.agent_id, &self.turn_id);
            return attempt;
        };
        let message = launch.text.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(message.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let mut stderr_task = tokio::spawn(read_stderr(stderr));

        let mut parser = StreamParser::new(self.redactor.clone());
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();
        loop {
            let limit = if self.step_inputs.is_empty() {
                launch.stall_timeout
            } else {
                launch.tool_stall_timeout
            };
            let read = match timeout(limit, read_line(&mut reader, &mut buffer)).await {
                Ok(read) => read,
                Err(_) => {
                    attempt.failure = Some(stall_message(limit));
                    if let Some(group) = self.registry.terminate(&self.agent_id, &self.turn_id) {
                        tokio::spawn(kill_group_after(group, EXIT_TERM_GRACE));
                    }
                    break;
                }
            };
            match read {
                Ok(Line::Text(line)) => {
                    for parsed in parser.line(&line) {
                        self.handle(parsed, &mut attempt);
                    }
                    if attempt.result.is_some() {
                        break;
                    }
                }
                Ok(Line::End) => break,
                Ok(Line::TooLong) => {
                    attempt.failure =
                        Some("Claude Code sent a line that is too long to read.".to_owned());
                    self.registry.kill(&self.agent_id, &self.turn_id);
                    break;
                }
                Err(_) => {
                    attempt.failure = Some("Could not read the output of Claude Code.".to_owned());
                    self.registry.kill(&self.agent_id, &self.turn_id);
                    break;
                }
            }
        }
        drop(reader);
        if timeout(EXIT_GRACE, child.wait()).await.is_err() {
            self.registry.kill(&self.agent_id, &self.turn_id);
            let _ = child.wait().await;
        }
        self.registry.clear_group(&self.agent_id, &self.turn_id);
        match timeout(STDERR_GRACE, &mut stderr_task).await {
            Ok(Ok(summary)) => attempt.stderr = summary,
            _ => stderr_task.abort(),
        }
        attempt
    }
}

fn stall_message(limit: Duration) -> String {
    let minutes = limit.as_secs().div_ceil(60).max(1);
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    format!("No response from the provider for {minutes} {unit}.")
}

fn failure_of(attempt: &Attempt) -> String {
    if let Some(message) = &attempt.failure {
        return message.clone();
    }
    match &attempt.result {
        Some((_, text)) if !text.trim().is_empty() => text.clone(),
        Some(_) => "Claude Code reported an error.".to_owned(),
        None => attempt
            .stderr
            .last_line
            .clone()
            .unwrap_or_else(|| "Claude Code exited unexpectedly.".to_owned()),
    }
}

pub async fn run(
    db: &Database,
    sink: &dyn EventSink,
    registry: &TurnRegistry,
    launch: Launch,
    key: ApiKey,
) -> bool {
    let (agent_id, turn_id) = (launch.agent_id.clone(), launch.turn_id.clone());
    let advance = run_turn(db, sink, registry, launch, key).await;
    registry.clear(&agent_id, &turn_id);
    advance
}

async fn run_turn(
    db: &Database,
    sink: &dyn EventSink,
    registry: &TurnRegistry,
    launch: Launch,
    key: ApiKey,
) -> bool {
    let stored_session = match &launch.session {
        Session::New(id) | Session::Resume(id) => id.clone(),
    };
    let mut runner = Runner {
        db,
        sink,
        registry,
        redactor: key.redactor(),
        agent_id: launch.agent_id.clone(),
        group_id: launch.group.as_ref().map(|group| group.group_id.clone()),
        turn_id: launch.turn_id.clone(),
        clock: launch.started_at,
        next_position: 0,
        texts: HashMap::new(),
        step_inputs: HashMap::new(),
        stored_session,
        save_failed: false,
        input_unread: false,
        previous_seen: launch.group.as_ref().map_or(0, |group| group.previous_seen),
    };
    runner.emit(TurnEvent::Started);
    if !launch.workspace.is_dir() {
        runner.input_unread = true;
        let missing = if launch.group.is_some() {
            "The group folder is missing."
        } else {
            "The workspace folder is missing."
        };
        runner.finish("failed", Some(missing.to_owned()));
        return false;
    }
    if let Some(refusal) = key.refusal() {
        runner.input_unread = true;
        runner.finish("failed", Some(refusal));
        return false;
    }
    let mut session = launch.session.clone();
    let mut attempt = runner.attempt(&launch, &session, &key).await;
    if matches!(session, Session::Resume(_))
        && !attempt.saw_init
        && !attempt.succeeded()
        && attempt.session_missing()
        && !registry.stop_requested(&launch.agent_id, &launch.turn_id)
    {
        session = Session::New(uuid::Uuid::new_v4().to_string());
        runner.add_notice(RESUME_NOTICE);
        attempt = runner.attempt(&launch, &session, &key).await;
    }
    if registry.is_closing() {
        runner.flush_texts();
        return false;
    }
    runner.input_unread = !attempt.saw_init;
    if matches!(session, Session::New(_)) && !attempt.saw_init {
        runner.store_session(None);
    }
    if attempt.succeeded() {
        runner.finish("completed", None);
        true
    } else if registry.stop_requested(&launch.agent_id, &launch.turn_id) {
        runner.finish("stopped", None);
        true
    } else {
        runner.finish("failed", Some(failure_of(&attempt)));
        attempt.spawned
    }
}

#[cfg(test)]
mod tests {
    use super::super::{STALL_TIMEOUT, TOOL_STALL_TIMEOUT};
    use super::*;
    use crate::api_key::key_for_turn;
    use crate::secrets::{SecretError, SecretStore};
    use crate::turns::claude_code::assert_isolated;
    use crate::turns::store::{self as turn_store, ItemView};
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    const BASIC: &str = include_str!("../../tests/fixtures/claude-stream-basic.jsonl");
    const DENIED: &str = include_str!("../../tests/fixtures/claude-stream-denied.jsonl");

    #[derive(Default)]
    struct Recorder(Mutex<Vec<TurnEnvelope>>);

    impl EventSink for Recorder {
        fn emit(&self, envelope: &TurnEnvelope) {
            self.0.lock().unwrap().push(envelope.clone());
        }
    }

    impl Recorder {
        fn kinds(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|e| {
                    serde_json::to_value(&e.event).unwrap()["kind"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect()
        }
    }

    struct World {
        data: tempfile::TempDir,
        bin: tempfile::TempDir,
        workspace: tempfile::TempDir,
        records: tempfile::TempDir,
        environment: Mutex<Vec<(OsString, OsString)>>,
        db: Database,
        sink: Recorder,
        registry: TurnRegistry,
    }

    impl World {
        fn new() -> Self {
            let data = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let db = Database::open(data.path()).unwrap();
            let path = workspace.path().to_string_lossy().into_owned();
            db.with_connection(|c| {
                c.execute(
                    "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                     VALUES ('a', 'Ada', 'red', 'claude-code', 'haiku', ?1, 1, 1)",
                    [path],
                )
                .unwrap();
            });
            Self {
                data,
                bin: tempfile::tempdir().unwrap(),
                workspace,
                records: tempfile::tempdir().unwrap(),
                environment: Mutex::new(Vec::new()),
                db,
                sink: Recorder::default(),
                registry: TurnRegistry::default(),
            }
        }

        fn install(&self, body: &str) {
            let path = self.bin.path().join("claude");
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn fixture(&self, name: &str, content: &str) -> PathBuf {
            let path = self.records.path().join(name);
            fs::write(&path, content).unwrap();
            path
        }

        fn set_environment(&self, name: &str, value: &str) {
            self.environment
                .lock()
                .unwrap()
                .push((name.into(), value.into()));
        }

        fn records(&self) -> &Path {
            self.records.path()
        }

        fn launch(&self, text: &str) -> Launch {
            let begun = self
                .db
                .with_connection(|c| turn_store::begin_turn(c, "a", text, now_millis()))
                .unwrap();
            Launch {
                agent_id: "a".into(),
                turn_id: begun.turn_id,
                executable: self.bin.path().join("claude"),
                search_dirs: vec![
                    self.bin.path().to_path_buf(),
                    "/usr/bin".into(),
                    "/bin".into(),
                ],
                workspace: self.workspace.path().to_path_buf(),
                model: begun.agent.model,
                session: begun.session,
                text: text.to_owned(),
                started_at: now_millis(),
                environment: self.environment.lock().unwrap().clone(),
                stall_timeout: STALL_TIMEOUT,
                tool_stall_timeout: TOOL_STALL_TIMEOUT,
                group: None,
            }
        }

        async fn send(&self, text: &str, key: ApiKey) -> String {
            let launch = self.launch(text);
            let turn_id = launch.turn_id.clone();
            run(&self.db, &self.sink, &self.registry, launch, key).await;
            turn_id
        }

        fn turns(&self) -> Vec<turn_store::TurnView> {
            self.db
                .with_connection(|c| turn_store::conversation(c, "a", None))
                .unwrap()
                .turns
        }

        fn session(&self) -> Option<String> {
            self.db.with_connection(|c| {
                c.query_row("SELECT provider_session_id FROM agents", [], |r| r.get(0))
                    .unwrap()
            })
        }
    }

    fn recording(world: &World, rest: &str) -> String {
        let records = world.records().display();
        format!(
            "n=$(ls \"{records}\" | grep -c '^args' || true)\nfor a in \"$@\"; do echo \"$a\"; done > \"{records}/args.$n\"\ncat > \"{records}/stdin.$n\"\nprintf '%s' \"$ANTHROPIC_API_KEY\" > \"{records}/key.$n\"\nprintf '%s' \"$ANTHROPIC_AUTH_TOKEN\" > \"{records}/token.$n\"\nprintf '%s' \"$ANTHROPIC_BASE_URL\" > \"{records}/base.$n\"\nprintf '%s' \"$LC_KEPT\" > \"{records}/kept.$n\"\n{rest}"
        )
    }

    fn read(world: &World, name: &str) -> String {
        fs::read_to_string(world.records().join(name)).unwrap()
    }

    fn args(world: &World, n: usize) -> Vec<String> {
        read(world, &format!("args.{n}"))
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn has_record(world: &World, name: &str) -> bool {
        world.records().join(name).exists()
    }

    fn only_turn(world: &World) -> turn_store::TurnView {
        let mut turns = world.turns();
        assert_eq!(turns.len(), 1);
        turns.remove(0)
    }

    const FIXTURE_SESSION: &str = "69dbf927-5f35-4e12-b4f6-07074d11d933";

    #[tokio::test]
    async fn a_first_turn_passes_a_new_session_id_and_the_message_arrives_on_stdin() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("Reply with the word pong", ApiKey::Subscription)
            .await;
        let args = args(&world, 0);
        assert_eq!(
            args[..5],
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ]
        );
        assert_isolated(&args);
        assert_eq!(args[12..14], ["--model", "haiku"]);
        assert_eq!(args[14], "--session-id");
        assert_eq!(args.len(), 16);
        assert_eq!(args[15].len(), 36);
        assert_ne!(args[15], FIXTURE_SESSION);
        assert!(!args.iter().any(|arg| arg.contains("pong")));
        assert_eq!(read(&world, "stdin.0"), "Reply with the word pong");
        assert_eq!(world.session().as_deref(), Some(FIXTURE_SESSION));
        assert!(world.registry.slots().is_empty());
    }

    #[tokio::test]
    async fn a_later_turn_resumes_the_stored_session() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("one", ApiKey::Subscription).await;
        world.send("two", ApiKey::Subscription).await;
        let second = args(&world, 1);
        assert_isolated(&second);
        assert_eq!(second[14..], ["--resume", FIXTURE_SESSION]);
        assert!(!second.iter().any(|arg| arg == "--session-id"));
        assert_eq!(world.turns().len(), 2);
    }

    #[tokio::test]
    async fn the_default_model_passes_no_model_flag() {
        let world = World::new();
        world
            .db
            .with_connection(|c| c.execute("UPDATE agents SET model = 'default'", []))
            .unwrap();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("hi", ApiKey::Subscription).await;
        let args = args(&world, 0);
        assert!(!args.iter().any(|arg| arg == "--model"));
        assert_isolated(&args);
    }

    #[tokio::test]
    async fn the_child_sees_the_saved_key_only_when_one_is_passed() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.set_environment("ANTHROPIC_API_KEY", "sk-ant-from-the-parent");
        world.set_environment("ANTHROPIC_AUTH_TOKEN", "token-from-the-parent");
        world.set_environment("ANTHROPIC_BASE_URL", "https://proxy.example");
        world.set_environment("LC_KEPT", "kept");
        world
            .send(
                "one",
                ApiKey::Present(SecretString::new("sk-ant-saved-0000")),
            )
            .await;
        world.send("two", ApiKey::Subscription).await;
        assert_eq!(read(&world, "key.0"), "sk-ant-saved-0000");
        assert_eq!(read(&world, "token.0"), "");
        assert_eq!(read(&world, "base.0"), "");
        assert_eq!(read(&world, "kept.0"), "kept");
        assert_eq!(read(&world, "key.1"), "");
        assert_eq!(read(&world, "token.1"), "");
        assert_eq!(read(&world, "base.1"), "");
    }

    #[tokio::test]
    async fn the_child_environment_holds_only_allowlisted_variables() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!(
                "env > \"{}/env.txt\"\ncat \"{}\"",
                world.records().display(),
                fixture.display()
            ),
        ));
        let allowed = [
            "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "TERM", "LC_ALL", "LC_CTYPE",
        ];
        let dropped = [
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDECODE",
            "CLAUDE_CONFIG_DIR",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "LCX",
            "XLC_ALL",
            "SHIKIGAMI_OTHER",
        ];
        for name in allowed.iter().chain(dropped.iter()) {
            world.set_environment(name, "value");
        }
        world.send("hi", ApiKey::Subscription).await;
        let text = read(&world, "env.txt");
        let mut names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .filter(|name| !["PWD", "OLDPWD", "SHLVL", "_"].contains(name))
            .collect();
        names.sort_unstable();
        let mut expected: Vec<&str> = allowed.to_vec();
        expected.push("PATH");
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[tokio::test]
    async fn the_saved_key_is_the_only_anthropic_variable_in_api_key_mode() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!(
                "env > \"{}/env.txt\"\ncat \"{}\"",
                world.records().display(),
                fixture.display()
            ),
        ));
        world.set_environment("ANTHROPIC_BASE_URL", "https://proxy.example");
        world.set_environment("ANTHROPIC_AUTH_TOKEN", "token");
        world
            .send(
                "one",
                ApiKey::Present(SecretString::new("sk-ant-saved-0000")),
            )
            .await;
        let text = read(&world, "env.txt");
        let anthropic: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("ANTHROPIC_"))
            .collect();
        assert_eq!(anthropic, ["ANTHROPIC_API_KEY=sk-ant-saved-0000"]);
    }

    #[tokio::test]
    async fn an_unreadable_key_fails_the_turn_and_starts_no_process() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("hi", ApiKey::Unreadable(None)).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("Could not read the saved API key.")
        );
        assert!(!has_record(&world, "args.0"));
        assert_eq!(world.sink.kinds(), ["started", "ended"]);
    }

    #[tokio::test]
    async fn an_unreadable_key_with_a_keychain_status_shows_the_code_in_the_turn_error() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let key = ApiKey::from_read(
            AuthMode::ApiKey,
            Err(ApiKeyError::from(SecretError::Read(Some(-25293)))),
        );
        world.send("hi", key).await;
        assert_eq!(
            only_turn(&world).error.as_deref(),
            Some("Could not read the saved API key (Keychain error -25293).")
        );
        assert!(!has_record(&world, "args.0"));
    }

    #[tokio::test]
    async fn no_saved_key_in_api_key_mode_fails_the_turn_and_starts_no_process() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let launch = world.launch("hi");
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            launch,
            ApiKey::from_read(AuthMode::ApiKey, Ok(None)),
        )
        .await;
        assert!(!advance);
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("No API key is saved. Add one or switch this agent to your Claude subscription.")
        );
        assert!(!has_record(&world, "args.0"));
        assert_eq!(world.sink.kinds(), ["started", "ended"]);
    }

    #[tokio::test]
    async fn an_unresolved_sign_in_setting_fails_the_turn_and_starts_no_process() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("hi", ApiKey::Unresolved).await;
        assert_eq!(
            only_turn(&world).error.as_deref(),
            Some("Could not read the sign-in setting.")
        );
        assert!(!has_record(&world, "args.0"));
    }

    #[tokio::test]
    async fn a_missing_workspace_fails_at_once_without_starting_the_process() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let launch = world.launch("hi");
        fs::remove_dir_all(world.workspace.path()).unwrap();
        run(
            &world.db,
            &world.sink,
            &world.registry,
            launch,
            ApiKey::Subscription,
        )
        .await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("The workspace folder is missing.")
        );
        assert!(!has_record(&world, "args.0"));
    }

    #[tokio::test]
    async fn the_basic_fixture_saves_and_streams_the_reply() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("Reply with the word pong", ApiKey::Subscription)
            .await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "completed");
        assert_eq!(turn.error, None);
        assert_eq!(turn.user_message, "Reply with the word pong");
        assert!(turn.ended_at.is_some());
        assert!(matches!(&turn.items[..], [ItemView::Text { text, .. }] if text == "pong"));
        assert_eq!(world.sink.kinds(), ["started", "text-set", "ended"]);
    }

    #[tokio::test]
    async fn the_denied_tool_fixture_saves_failed_steps_before_the_answer() {
        let world = World::new();
        let fixture = world.fixture("denied.jsonl", DENIED);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("List the files in your folder", ApiKey::Subscription)
            .await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "completed");
        let [ItemView::Step(first), ItemView::Step(second), ItemView::Text { text, .. }] =
            &turn.items[..]
        else {
            panic!("unexpected items: {:?}", turn.items);
        };
        assert_eq!(first.kind, "Bash");
        assert!(first.title.starts_with("ls -la"));
        assert_eq!(first.status, "failed");
        assert!(first.detail.as_deref().unwrap().contains("\"command\""));
        assert!(second.title.starts_with("/bin/ls -la"));
        assert_eq!(second.status, "failed");
        assert!(second.detail.as_deref().unwrap().contains("denied"));
        assert!(!text.is_empty());
        let kinds = world.sink.kinds();
        assert_eq!(kinds.first().map(String::as_str), Some("started"));
        assert_eq!(kinds.last().map(String::as_str), Some("ended"));
        for kind in ["step-started", "step-ended", "text-set"] {
            assert!(kinds.iter().any(|k| k == kind), "{kind}");
        }
    }

    #[tokio::test]
    async fn a_tool_result_without_an_error_saves_a_done_step() {
        let world = World::new();
        let succeeding = DENIED.replace("\"is_error\":true", "\"is_error\":false");
        let fixture = world.fixture("denied.jsonl", &succeeding);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("List the files", ApiKey::Subscription).await;
        let turn = only_turn(&world);
        assert!(matches!(
            &turn.items[..],
            [ItemView::Step(first), ItemView::Step(second), ItemView::Text { .. }]
                if first.status == "done" && second.status == "done"
        ));
    }

    #[tokio::test]
    async fn an_error_result_fails_the_turn_with_its_text() {
        let world = World::new();
        let failing = BASIC.replace("\"is_error\":false", "\"is_error\":true");
        let fixture = world.fixture("basic.jsonl", &failing);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("hi", ApiKey::Subscription).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(turn.error.as_deref(), Some("pong"));
    }

    #[tokio::test]
    async fn exiting_without_a_result_fails_with_the_last_stderr_line() {
        let world = World::new();
        world.install(&recording(
            &world,
            "echo 'first' >&2\necho 'boom: bad thing' >&2\nexit 3",
        ));
        world.send("hi", ApiKey::Subscription).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(turn.error.as_deref(), Some("boom: bad thing"));
        assert_eq!(world.sink.kinds().last().map(String::as_str), Some("ended"));
    }

    #[tokio::test]
    async fn exiting_silently_gives_the_generic_message_and_the_key_is_never_echoed() {
        let world = World::new();
        world.install(&recording(&world, "exit 1"));
        world.send("hi", ApiKey::Subscription).await;
        assert_eq!(
            only_turn(&world).error.as_deref(),
            Some("Claude Code exited unexpectedly.")
        );
        let leaky = World::new();
        leaky.install(&recording(
            &leaky,
            "echo \"bad key $ANTHROPIC_API_KEY\" >&2\nexit 1",
        ));
        leaky
            .send(
                "hi",
                ApiKey::Present(SecretString::new("sk-ant-secret-1234")),
            )
            .await;
        let error = only_turn(&leaky).error.unwrap();
        assert_eq!(error, "bad key [redacted]");
    }

    #[tokio::test]
    async fn a_line_over_one_mebibyte_fails_the_turn() {
        let world = World::new();
        world.install(&recording(
            &world,
            "head -c 1200000 /dev/zero | tr '\\0' a\necho",
        ));
        world.send("hi", ApiKey::Subscription).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert!(turn.error.unwrap().contains("too long"));
        assert!(world.registry.slots().is_empty());
    }

    const MISSING: &str = "echo 'No conversation found with session ID: old-id' >&2\nexit 1";

    const FRAGMENT_KEY: &str = "sk-ant-api03-FRAGMENT0123456789ABCDEFGHIJ";

    fn stderr_cut_inside_the_key(tail: &str) -> Vec<u8> {
        let head = format!("start {FRAGMENT_KEY}");
        let cut_inside_key = "start sk-ant-api03-".len() + 8;
        let filler = STDERR_KEEP + cut_inside_key - head.len() - tail.len();
        format!("{head}{}{tail}", "x".repeat(filler)).into_bytes()
    }

    #[tokio::test]
    async fn a_trimmed_stderr_shows_no_fragment_of_a_key_in_its_partial_first_line() {
        let data = stderr_cut_inside_the_key("");
        assert_eq!(data.len(), STDERR_KEEP + "start sk-ant-api03-".len() + 8);
        let summary = read_stderr(&data[..]).await;
        assert_eq!(summary.last_line, None);
        let data = stderr_cut_inside_the_key("\nreal error\n");
        let summary = read_stderr(&data[..]).await;
        assert_eq!(summary.last_line.as_deref(), Some("real error"));
    }

    #[tokio::test]
    async fn an_untrimmed_stderr_keeps_its_first_line() {
        let summary = read_stderr(&b"only line\n"[..]).await;
        assert_eq!(summary.last_line.as_deref(), Some("only line"));
    }

    fn stored_session_probe(world: &World) -> String {
        format!(
            "sqlite3 \"{}/shikigami.db\" 'SELECT COALESCE(provider_session_id, \"NULL\") FROM agents' > \"{}/stored.$n\"",
            world.data.path().display(),
            world.records().display()
        )
    }

    fn with_old_session(world: &World) {
        world
            .db
            .with_connection(|c| c.execute("UPDATE agents SET provider_session_id = 'old-id'", []))
            .unwrap();
    }

    #[tokio::test]
    async fn a_missing_session_starts_a_new_one_once_and_adds_a_notice() {
        let world = World::new();
        with_old_session(&world);
        let fixture = world.fixture("basic.jsonl", BASIC);
        let body = recording(
            &world,
            &format!(
                "{}\ncase \"$*\" in *--resume*) {MISSING};; esac\ncat \"{}\"",
                stored_session_probe(&world),
                fixture.display()
            ),
        );
        world.install(&body);
        world.send("hello again", ApiKey::Subscription).await;
        let first = args(&world, 0);
        let second = args(&world, 1);
        assert_isolated(&second);
        assert_eq!(first[14..], ["--resume", "old-id"]);
        assert_eq!(second[14], "--session-id");
        assert_ne!(second[15], "old-id");
        assert_eq!(read(&world, "stdin.1"), "hello again");
        assert_eq!(read(&world, "stored.1").trim(), "old-id");
        assert!(!has_record(&world, "args.2"));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "completed");
        assert!(matches!(
            &turn.items[..],
            [ItemView::Notice { text, .. }, ItemView::Text { .. }] if text == RESUME_NOTICE
        ));
        assert_eq!(world.session().as_deref(), Some(FIXTURE_SESSION));
        assert!(world.sink.kinds().contains(&"notice".to_owned()));
    }

    #[tokio::test]
    async fn a_missing_session_in_the_result_text_also_starts_a_new_one() {
        let world = World::new();
        with_old_session(&world);
        let fixture = world.fixture("basic.jsonl", BASIC);
        let result = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"No conversation found with session ID: old-id"}"#;
        world.install(&recording(
            &world,
            &format!(
                "case \"$*\" in *--resume*) echo '{result}'; exit 1;; esac\ncat \"{}\"",
                fixture.display()
            ),
        ));
        world.send("hello again", ApiKey::Subscription).await;
        assert!(has_record(&world, "args.1"));
        assert_eq!(only_turn(&world).status, "completed");
    }

    #[tokio::test]
    async fn a_retry_that_fails_again_is_not_retried_and_clears_the_session() {
        let world = World::new();
        with_old_session(&world);
        world.install(&recording(&world, MISSING));
        world.send("hi", ApiKey::Subscription).await;
        assert!(has_record(&world, "args.1"));
        assert!(!has_record(&world, "args.2"));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("No conversation found with session ID: old-id")
        );
        assert_eq!(world.session(), None);
    }

    #[tokio::test]
    async fn another_resume_failure_is_not_retried_and_keeps_the_session() {
        let world = World::new();
        with_old_session(&world);
        world.install(&recording(&world, "echo 'nope' >&2\nexit 1"));
        world.send("hi", ApiKey::Subscription).await;
        assert!(!has_record(&world, "args.1"));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(turn.error.as_deref(), Some("nope"));
        assert_eq!(world.session().as_deref(), Some("old-id"));
        assert!(turn.items.is_empty());
    }

    #[tokio::test]
    async fn a_new_session_that_never_starts_is_not_kept() {
        let world = World::new();
        world.install(&recording(&world, "echo 'nope' >&2\nexit 1"));
        world.send("hi", ApiKey::Subscription).await;
        assert_eq!(only_turn(&world).status, "failed");
        assert_eq!(world.session(), None);
        let args = args(&world, 0);
        assert_eq!(args[14], "--session-id");
    }

    #[tokio::test]
    async fn a_failure_after_the_session_started_is_not_treated_as_a_resume_failure() {
        let world = World::new();
        world
            .db
            .with_connection(|c| c.execute("UPDATE agents SET provider_session_id = 'old-id'", []))
            .unwrap();
        let init = r#"{"type":"system","subtype":"init","session_id":"old-id"}"#;
        world.install(&recording(
            &world,
            &format!("echo '{init}'\necho 'died' >&2\nexit 1"),
        ));
        world.send("hi", ApiKey::Subscription).await;
        assert!(!has_record(&world, "args.1"));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert!(turn.items.is_empty());
        assert_eq!(world.session().as_deref(), Some("old-id"));
    }

    const SAVED_KEY: &str = "sk-ant-api03-SAVEDSAVEDSAVEDSAVEDSAVEDSAVEDSAVED";

    fn stream_lines(lines: &[serde_json::Value]) -> String {
        lines.iter().map(|line| format!("{line}\n")).collect()
    }

    fn delta(index: u64, text: &str) -> serde_json::Value {
        serde_json::json!({"type":"stream_event","api_message_id":"m1","event":{"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}}})
    }

    fn everything_emitted(world: &World) -> String {
        let events = world.sink.0.lock().unwrap();
        serde_json::to_string(&*events).unwrap()
    }

    #[tokio::test]
    async fn a_key_in_a_tool_step_a_reply_and_an_error_is_stored_and_emitted_redacted() {
        let world = World::new();
        let lines = stream_lines(&[
            serde_json::json!({"type":"system","subtype":"init","session_id":"s1"}),
            serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":format!("echo {SAVED_KEY}")}}]}}),
            serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":format!("out {SAVED_KEY} and sk-ant-another-token-0123456789"),"is_error":false}]}}),
            serde_json::json!({"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":format!("the key is {SAVED_KEY}")}]}}),
            serde_json::json!({"type":"result","is_error":true,"result":format!("failed with {SAVED_KEY}")}),
        ]);
        let fixture = world.fixture("leak.jsonl", &lines);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("hi", ApiKey::Present(SecretString::new(SAVED_KEY)))
            .await;
        let turn = only_turn(&world);
        let stored = serde_json::to_string(&turn).unwrap();
        assert!(!stored.contains("SAVEDSAVED") && !stored.contains("another-token"));
        assert!(!everything_emitted(&world).contains("SAVEDSAVED"));
        assert!(!everything_emitted(&world).contains("another-token"));
        let [ItemView::Step(step), ItemView::Text { text, .. }] = &turn.items[..] else {
            panic!("unexpected items: {:?}", turn.items);
        };
        assert_eq!(step.title, "echo [redacted]");
        let detail = step.detail.as_deref().unwrap();
        assert!(
            detail.contains("echo [redacted]") && detail.contains("out [redacted] and [redacted]")
        );
        assert_eq!(text, "the key is [redacted]");
        assert_eq!(turn.error.as_deref(), Some("failed with [redacted]"));
    }

    #[tokio::test]
    async fn a_key_split_across_streamed_deltas_is_never_emitted_whole_or_in_part() {
        let world = World::new();
        let (head, tail) = SAVED_KEY.split_at(25);
        let padding = "p".repeat(300);
        let lines = stream_lines(&[
            serde_json::json!({"type":"system","subtype":"init","session_id":"s1"}),
            delta(0, &format!("{padding} {head}")),
            delta(0, tail),
            delta(0, &format!(" {padding}")),
            serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":format!("{padding} {SAVED_KEY} {padding}")}]}}),
            serde_json::json!({"type":"result","is_error":false,"result":"ok"}),
        ]);
        let fixture = world.fixture("split.jsonl", &lines);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("hi", ApiKey::Present(SecretString::new(SAVED_KEY)))
            .await;
        let emitted = everything_emitted(&world);
        assert!(!emitted.contains("sk-ant"), "{emitted}");
        assert!(!emitted.contains("SAVED"));
        let turn = only_turn(&world);
        assert!(
            matches!(&turn.items[..], [ItemView::Text { text, .. }] if *text == format!("{padding} [redacted] {padding}"))
        );
        assert!(world.sink.kinds().contains(&"text-delta".to_owned()));
    }

    #[tokio::test]
    async fn a_key_shaped_token_longer_than_the_hold_back_is_never_emitted_in_part() {
        let world = World::new();
        let token = format!("sk-ant-{}", "A".repeat(130));
        let lines = stream_lines(&[
            serde_json::json!({"type":"system","subtype":"init","session_id":"s1"}),
            delta(0, &format!("{}sk-ant-", "p".repeat(100))),
            delta(0, &"A".repeat(130)),
            serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":format!("{}{token}", "p".repeat(100))}]}}),
            serde_json::json!({"type":"result","is_error":false,"result":"ok"}),
        ]);
        let fixture = world.fixture("long.jsonl", &lines);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("hi", ApiKey::Subscription).await;
        assert!(!everything_emitted(&world).contains("sk-ant"));
        let turn = only_turn(&world);
        assert!(
            matches!(&turn.items[..], [ItemView::Text { text, .. }] if text.ends_with("pp[redacted]"))
        );
    }

    #[tokio::test]
    async fn text_streamed_without_a_final_message_is_flushed_redacted_when_the_turn_ends() {
        let world = World::new();
        let lines = stream_lines(&[
            serde_json::json!({"type":"system","subtype":"init","session_id":"s1"}),
            delta(0, &format!("see {SAVED_KEY}")),
        ]);
        let fixture = world.fixture("cut.jsonl", &lines);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("hi", ApiKey::Present(SecretString::new(SAVED_KEY)))
            .await;
        let turn = only_turn(&world);
        assert!(
            matches!(&turn.items[..], [ItemView::Text { text, .. }] if text == "see [redacted]")
        );
        assert!(!everything_emitted(&world).contains("SAVEDSAVED"));
    }

    fn partial_stream() -> String {
        stream_lines(&[
            delta(0, &format!("partial {SAVED_KEY}")),
            serde_json::json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"sleep 99"}}]}}),
        ])
    }

    async fn run_and_stop(world: &World, grace: Duration) -> (bool, Duration) {
        let launch = world.launch("go");
        let turn_id = launch.turn_id.clone();
        assert!(world.registry.begin("a", &turn_id));
        let started = Instant::now();
        let stopper = async {
            while !world.sink.kinds().iter().any(|kind| kind == "step-started") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let target = world.registry.request_stop("a", &turn_id).unwrap();
            kill_group_after(target.group.unwrap(), grace).await;
        };
        let (advance, ()) = timeout(Duration::from_secs(10), async {
            tokio::join!(
                run(
                    &world.db,
                    &world.sink,
                    &world.registry,
                    launch,
                    ApiKey::Present(SecretString::new(SAVED_KEY)),
                ),
                stopper
            )
        })
        .await
        .expect("a stopped turn must end");
        (advance, started.elapsed())
    }

    fn install_streaming_then(world: &World, before: &str, rest: &str) {
        let fixture = world.fixture("partial.jsonl", &partial_stream());
        world.install(&recording(
            world,
            &format!("{before}\ncat \"{}\"\n{rest}", fixture.display()),
        ));
    }

    #[tokio::test]
    async fn a_stop_ends_the_turn_stopped_and_keeps_the_partial_reply_redacted() {
        let world = World::new();
        install_streaming_then(&world, "", "sleep 30");
        let grace = Duration::from_secs(2);
        let (advance, elapsed) = run_and_stop(&world, grace).await;
        assert!(advance);
        assert!(elapsed < grace + LOAD_SLACK);
        let turn = only_turn(&world);
        assert_eq!(turn.status, "stopped");
        assert_eq!(turn.error, None);
        assert!(turn.ended_at.is_some());
        assert!(matches!(
            &turn.items[..],
            [ItemView::Text { text, .. }, ItemView::Step(step)]
                if text == "partial [redacted]" && step.status == "stopped"
        ));
        assert!(!everything_emitted(&world).contains("SAVEDSAVED"));
        let events = world.sink.0.lock().unwrap();
        assert!(matches!(
            events.last().map(|envelope| &envelope.event),
            Some(TurnEvent::Ended { status, error: None }) if status == "stopped"
        ));
        drop(events);
        assert!(!has_record(&world, "args.1"));
        assert!(world.registry.slots().is_empty());
    }

    #[tokio::test]
    async fn a_child_that_ignores_the_term_signal_is_killed_after_the_grace() {
        let world = World::new();
        install_streaming_then(&world, "", "trap '' TERM\nwhile true; do sleep 1; done");
        let (_, elapsed) = run_and_stop(&world, Duration::from_millis(300)).await;
        assert!(elapsed + TIMER_SKEW >= Duration::from_millis(300));
        assert!(elapsed < Duration::from_millis(300) + LOAD_SLACK);
        assert_eq!(only_turn(&world).status, "stopped");
    }

    #[tokio::test]
    async fn a_stop_is_never_retried_as_a_missing_session() {
        let world = World::new();
        with_old_session(&world);
        install_streaming_then(&world, "echo 'No conversation found' >&2", "sleep 30");
        run_and_stop(&world, Duration::from_secs(2)).await;
        assert_eq!(only_turn(&world).status, "stopped");
        assert!(has_record(&world, "args.0"));
        assert!(!has_record(&world, "args.1"));
        let turn = only_turn(&world);
        assert!(!turn
            .items
            .iter()
            .any(|item| matches!(item, ItemView::Notice { .. })));
    }

    #[tokio::test]
    async fn a_stop_requested_before_the_spawn_ends_stopped_without_starting_a_process() {
        let world = World::new();
        let launch = world.launch("go");
        assert!(world.registry.begin("a", &launch.turn_id));
        assert!(world.registry.request_stop("a", &launch.turn_id).is_some());
        let started = Instant::now();
        let advance = timeout(
            Duration::from_secs(10),
            run(
                &world.db,
                &world.sink,
                &world.registry,
                launch,
                ApiKey::Subscription,
            ),
        )
        .await
        .expect("a stopped turn must end");
        assert!(advance);
        assert!(started.elapsed() < LOAD_SLACK);
        let turn = only_turn(&world);
        assert_eq!(turn.status, "stopped");
        assert_eq!(turn.error, None);
    }

    #[tokio::test]
    async fn a_turn_stopped_by_another_turns_request_is_not_affected() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let launch = world.launch("go");
        assert!(world.registry.begin("a", &launch.turn_id));
        assert!(world.registry.request_stop("a", "an-older-turn").is_none());
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            launch,
            ApiKey::Subscription,
        )
        .await;
        assert!(advance);
        assert_eq!(only_turn(&world).status, "completed");
    }

    #[tokio::test]
    async fn a_closing_app_starts_no_process_and_leaves_the_turn_for_the_startup_sweep() {
        let world = World::new();
        world.install(&recording(&world, "sleep 30"));
        world.registry.closing.store(true, Ordering::SeqCst);
        let launch = world.launch("go");
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            launch,
            ApiKey::Subscription,
        )
        .await;
        assert!(!advance);
        assert!(!has_record(&world, "args.0"));
        assert_eq!(only_turn(&world).status, "running");
        assert!(!world.sink.kinds().iter().any(|kind| kind == "ended"));
    }

    #[tokio::test]
    async fn a_turn_ended_by_the_shutdown_kill_stays_running_and_does_not_advance() {
        let world = World::new();
        install_streaming_then(&world, "", "sleep 30");
        let launch = world.launch("go");
        let turn_id = launch.turn_id.clone();
        assert!(world.registry.begin("a", &turn_id));
        let closer = async {
            while !world.sink.kinds().iter().any(|kind| kind == "step-started") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            world.registry.closing.store(true, Ordering::SeqCst);
            world.registry.request_stop("a", &turn_id).unwrap();
        };
        let (advance, ()) = timeout(Duration::from_secs(10), async {
            tokio::join!(
                run(
                    &world.db,
                    &world.sink,
                    &world.registry,
                    launch,
                    ApiKey::Subscription,
                ),
                closer
            )
        })
        .await
        .expect("the killed turn must end");
        assert!(!advance);
        assert_eq!(only_turn(&world).status, "running");
        assert!(!world.sink.kinds().iter().any(|kind| kind == "ended"));
    }

    #[tokio::test]
    async fn a_turn_that_fails_before_its_process_starts_does_not_advance_the_queue() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let missing_folder = world.launch("one");
        fs::remove_dir_all(world.workspace.path()).unwrap();
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            missing_folder,
            ApiKey::Subscription,
        )
        .await;
        assert!(!advance);
        fs::create_dir_all(world.workspace.path()).unwrap();
        let unreadable = world.launch("two");
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            unreadable,
            ApiKey::Unreadable(None),
        )
        .await;
        assert!(!advance);
        fs::remove_file(world.bin.path().join("claude")).unwrap();
        let no_provider = world.launch("three");
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            no_provider,
            ApiKey::Subscription,
        )
        .await;
        assert!(!advance);
        let turns = world.turns();
        assert_eq!(turns.len(), 3);
        assert!(turns.iter().all(|turn| turn.status == "failed"));
    }

    #[tokio::test]
    async fn a_turn_that_completes_or_fails_after_its_process_started_advances_the_queue() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let launch = world.launch("ok");
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            launch,
            ApiKey::Subscription,
        )
        .await;
        assert!(advance);
        world.install("echo broke >&2\nexit 1");
        let launch = world.launch("bad");
        let advance = run(
            &world.db,
            &world.sink,
            &world.registry,
            launch,
            ApiKey::Subscription,
        )
        .await;
        assert!(advance);
        let statuses: Vec<_> = world.turns().into_iter().map(|turn| turn.status).collect();
        assert_eq!(statuses, ["completed", "failed"]);
    }

    struct VanishedItem;

    impl SecretStore for VanishedItem {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            Ok(true)
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_saved_key_that_cannot_be_read_back_fails_the_turn_like_a_read_error() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        let key = ApiKey::from_read(
            AuthMode::ApiKey,
            key_for_turn(&VanishedItem, AuthMode::ApiKey, "claude-code"),
        );
        assert!(matches!(key, ApiKey::Unreadable(None)));
        world.send("hi", key).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("Could not read the saved API key.")
        );
        assert!(!has_record(&world, "args.0"));
    }

    const SHORT_STALL: Duration = Duration::from_millis(600);
    const SHORT_TOOL_STALL: Duration = Duration::from_millis(2500);
    const LONG_STALL: Duration = Duration::from_secs(60);
    const STEADY_STALL: Duration = Duration::from_millis(1800);
    const QUIET_TOOL_LIMIT: Duration = Duration::from_secs(5);
    const LOAD_SLACK: Duration = Duration::from_secs(5);
    const TIMER_SKEW: Duration = Duration::from_millis(50);

    async fn run_with_stall(world: &World, idle: Duration, tool: Duration) -> bool {
        let mut launch = world.launch("go");
        launch.stall_timeout = idle;
        launch.tool_stall_timeout = tool;
        assert!(world.registry.begin("a", &launch.turn_id));
        timeout(
            Duration::from_secs(15),
            run(
                &world.db,
                &world.sink,
                &world.registry,
                launch,
                ApiKey::Subscription,
            ),
        )
        .await
        .expect("a stalled turn must end")
    }

    fn install_quiet_then(world: &World, before: &str, rest: &str) {
        world.install(&recording(
            world,
            &format!("{before}\necho 'hello'\n{rest}"),
        ));
    }

    fn shell_pid(world: &World) -> i32 {
        fs::read_to_string(world.records().join("shell.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn record_pid(world: &World) -> String {
        format!("echo $$ > \"{}/shell.pid\"", world.records().display())
    }

    fn term_marker(world: &World) -> String {
        format!(
            "trap 'echo got-term > \"{}/term.marker\"; exit 143' TERM",
            world.records().display()
        )
    }

    #[test]
    fn the_stall_message_names_the_limit_that_applied() {
        assert_eq!(
            stall_message(STALL_TIMEOUT),
            "No response from the provider for 5 minutes."
        );
        assert_eq!(
            stall_message(TOOL_STALL_TIMEOUT),
            "No response from the provider for 15 minutes."
        );
        assert_eq!(
            stall_message(Duration::from_secs(1)),
            "No response from the provider for 1 minute."
        );
    }

    #[tokio::test]
    async fn a_turn_with_no_output_and_no_running_step_fails_after_the_idle_limit() {
        let world = World::new();
        install_quiet_then(
            &world,
            &format!("{}\n{}", record_pid(&world), term_marker(&world)),
            "sleep 60",
        );
        let started = Instant::now();
        let advance = run_with_stall(&world, SHORT_STALL, LONG_STALL).await;
        assert!(started.elapsed() < Duration::from_secs(8));
        assert!(advance);
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("No response from the provider for 1 minute.")
        );
        assert!(has_record(&world, "term.marker"));
        assert_ne!(unsafe { libc::killpg(shell_pid(&world), 0) }, 0);
        assert!(world.registry.slots().is_empty());
        assert_eq!(world.sink.kinds().last().map(String::as_str), Some("ended"));
    }

    #[tokio::test]
    async fn a_turn_with_a_running_step_waits_for_the_tool_limit_and_stops_the_step() {
        let world = World::new();
        install_streaming_then(
            &world,
            &format!("{}\n{}", record_pid(&world), term_marker(&world)),
            "sleep 60",
        );
        let started = Instant::now();
        let advance = run_with_stall(&world, SHORT_STALL, SHORT_TOOL_STALL).await;
        assert!(started.elapsed() + TIMER_SKEW >= SHORT_TOOL_STALL);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(advance);
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("No response from the provider for 1 minute.")
        );
        let steps: Vec<_> = turn
            .items
            .iter()
            .filter_map(|item| match item {
                ItemView::Step(step) => Some(step.status.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(steps, ["stopped"]);
        assert!(turn.items.iter().any(
            |item| matches!(item, ItemView::Text { text, .. } if text.starts_with("partial"))
        ));
        assert!(has_record(&world, "term.marker"));
        assert_ne!(unsafe { libc::killpg(shell_pid(&world), 0) }, 0);
    }

    #[tokio::test]
    async fn a_quiet_tool_that_ends_within_the_tool_limit_is_not_stalled() {
        let world = World::new();
        install_streaming_then(&world, "", "sleep 1.5\nexit 0");
        let started = Instant::now();
        run_with_stall(&world, SHORT_STALL, QUIET_TOOL_LIMIT).await;
        assert!(started.elapsed() + TIMER_SKEW >= Duration::from_millis(1500));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(
            turn.error.as_deref(),
            Some("Claude Code exited unexpectedly.")
        );
    }

    #[tokio::test]
    async fn a_stalled_provider_that_ignores_the_term_signal_is_killed_after_the_grace() {
        let world = World::new();
        install_quiet_then(
            &world,
            &format!("trap '' TERM\n{}", record_pid(&world)),
            "while true; do sleep 1; done",
        );
        let started = Instant::now();
        run_with_stall(&world, SHORT_STALL, LONG_STALL).await;
        let elapsed = started.elapsed();
        assert!(elapsed + TIMER_SKEW >= SHORT_STALL + EXIT_TERM_GRACE);
        assert!(elapsed < SHORT_STALL + EXIT_TERM_GRACE + LOAD_SLACK);
        assert_eq!(only_turn(&world).status, "failed");
        assert_ne!(unsafe { libc::killpg(shell_pid(&world), 0) }, 0);
    }

    #[tokio::test]
    async fn a_provider_that_keeps_printing_within_the_timeout_is_not_stalled() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!(
                "while IFS= read -r line; do printf '%s\\n' \"$line\"; sleep 0.3; done < \"{}\"",
                fixture.display()
            ),
        ));
        let started = Instant::now();
        run_with_stall(&world, STEADY_STALL, LONG_STALL).await;
        assert!(started.elapsed() > STEADY_STALL);
        let turn = only_turn(&world);
        assert_eq!(turn.status, "completed");
        assert_eq!(turn.error, None);
    }

    #[tokio::test]
    async fn output_that_the_parser_ignores_still_counts_as_activity() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!(
                "for i in 1 2 3 4 5 6 7 8 9 10; do echo 'not json at all'; sleep 0.3; done\ncat \"{}\"",
                fixture.display()
            ),
        ));
        let started = Instant::now();
        run_with_stall(&world, STEADY_STALL, LONG_STALL).await;
        assert!(started.elapsed() > STEADY_STALL);
        assert_eq!(only_turn(&world).status, "completed");
    }

    #[tokio::test]
    async fn a_provider_killed_mid_turn_fails_with_its_stderr_line_and_the_next_message_works() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!(
                "head -n 3 \"{}\"\necho 'provider crashed: out of memory' >&2\nkill -9 $$",
                fixture.display()
            ),
        ));
        world.send("first", ApiKey::Subscription).await;
        let crashed = only_turn(&world);
        assert_eq!(crashed.status, "failed");
        assert_eq!(
            crashed.error.as_deref(),
            Some("provider crashed: out of memory")
        );
        assert!(world.registry.slots().is_empty());
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("second", ApiKey::Subscription).await;
        let statuses: Vec<_> = world.turns().into_iter().map(|turn| turn.status).collect();
        assert_eq!(statuses, ["failed", "completed"]);
    }
}
