use std::collections::HashMap;
use std::env;
use std::io;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::claude_code::{self, Parsed, Session, StepStart, StreamParser};
use super::store::{self, StepView};
use super::{EventSink, Launch, TurnEnvelope, TurnEvent, TurnRegistry};
use crate::agents::now_millis;
use crate::db::Database;
use crate::secrets::SecretString;

const LINE_LIMIT: usize = 1024 * 1024;
const STDERR_KEEP: usize = 64 * 1024;
const STDERR_LINE_LIMIT: usize = 4096;
const TEXT_WRITE_INTERVAL: Duration = Duration::from_millis(500);
const EXIT_GRACE: Duration = Duration::from_secs(5);
const STDERR_GRACE: Duration = Duration::from_secs(2);

const RESUME_NOTICE: &str =
    "Started a new Claude Code session. The agent may have lost earlier context.";

pub enum ApiKey {
    Present(SecretString),
    Absent,
    Unreadable,
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

async fn last_stderr_line<R: AsyncReadExt + Unpin>(mut stderr: R) -> Option<String> {
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                kept.extend_from_slice(&chunk[..count]);
                if kept.len() > STDERR_KEEP {
                    kept.drain(..kept.len() - STDERR_KEEP);
                }
            }
        }
    }
    String::from_utf8_lossy(&kept)
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(|line| claude_code::truncate(line, STDERR_LINE_LIMIT))
}

struct TextRow {
    text: String,
    written: Instant,
    dirty: bool,
}

#[derive(Default)]
struct Attempt {
    spawned: bool,
    saw_init: bool,
    result: Option<(bool, String)>,
    failure: Option<String>,
    stderr_line: Option<String>,
}

impl Attempt {
    fn succeeded(&self) -> bool {
        matches!(self.result, Some((false, _))) && self.failure.is_none()
    }
}

struct Runner<'a> {
    db: &'a Database,
    sink: &'a dyn EventSink,
    registry: &'a TurnRegistry,
    agent_id: String,
    turn_id: String,
    clock: i64,
    next_position: i64,
    texts: HashMap<String, TextRow>,
    step_inputs: HashMap<String, String>,
    stored_session: String,
    save_failed: bool,
}

impl Runner<'_> {
    fn tick(&mut self) -> i64 {
        self.clock = now_millis().max(self.clock + 1);
        self.clock
    }

    fn emit(&self, event: TurnEvent) {
        self.sink.emit(&TurnEnvelope {
            agent_id: self.agent_id.clone(),
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
        let id = uuid::Uuid::now_v7().to_string();
        let created_at = self.tick();
        let (agent_id, turn_id) = (self.agent_id.clone(), self.turn_id.clone());
        self.save(|connection| {
            store::insert_message(
                connection, &id, &agent_id, &turn_id, "notice", text, created_at,
            )
        });
        self.emit(TurnEvent::Notice {
            item_id: id,
            text: text.to_owned(),
        });
    }

    fn write_text(&mut self, item_id: &str) {
        let Some(row) = self.texts.get_mut(item_id) else {
            return;
        };
        row.written = Instant::now();
        row.dirty = false;
        let text = row.text.clone();
        self.save(|connection| store::update_message(connection, item_id, &text));
    }

    fn ensure_text_row(&mut self, item_id: &str) {
        if self.texts.contains_key(item_id) {
            return;
        }
        let created_at = self.tick();
        let (agent_id, turn_id) = (self.agent_id.clone(), self.turn_id.clone());
        self.save(|connection| {
            store::insert_message(
                connection,
                item_id,
                &agent_id,
                &turn_id,
                "assistant",
                "",
                created_at,
            )
        });
        self.texts.insert(
            item_id.to_owned(),
            TextRow {
                text: String::new(),
                written: Instant::now(),
                dirty: false,
            },
        );
    }

    fn flush_texts(&mut self) {
        let dirty: Vec<String> = self
            .texts
            .iter()
            .filter(|(_, row)| row.dirty)
            .map(|(id, _)| id.clone())
            .collect();
        for id in dirty {
            self.write_text(&id);
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
                    let agent_id = self.agent_id.clone();
                    self.save(|connection| store::set_session(connection, &agent_id, Some(&id)));
                    self.stored_session = id;
                }
            }
            Parsed::TextDelta { item_id, text } => {
                self.ensure_text_row(&item_id);
                if let Some(row) = self.texts.get_mut(&item_id) {
                    row.text.push_str(&text);
                    row.dirty = true;
                    if row.written.elapsed() >= TEXT_WRITE_INTERVAL {
                        self.write_text(&item_id);
                    }
                }
                self.emit(TurnEvent::TextDelta { item_id, text });
            }
            Parsed::TextSet { item_id, text } => {
                self.ensure_text_row(&item_id);
                if let Some(row) = self.texts.get_mut(&item_id) {
                    row.text.clone_from(&text);
                }
                self.write_text(&item_id);
                self.emit(TurnEvent::TextSet { item_id, text });
            }
            Parsed::StepStarted(step) => self.start_step(step),
            Parsed::StepEnded {
                step_id,
                failed,
                output,
            } => self.end_step(step_id, failed, output),
            Parsed::Result { is_error, text } => attempt.result = Some((is_error, text)),
        }
    }

    fn finish(&mut self, status: &str, error: Option<String>) {
        self.flush_texts();
        let (status, error) = if status == "completed" && self.save_failed {
            ("failed", Some("Could not save the whole reply.".to_owned()))
        } else {
            (status, error)
        };
        let now = self.tick();
        let (turn_id, agent_id) = (self.turn_id.clone(), self.agent_id.clone());
        self.save(|connection| {
            store::finish_turn(
                connection,
                &turn_id,
                &agent_id,
                status,
                error.as_deref(),
                now,
            )
        });
        self.emit(TurnEvent::Ended {
            status: status.to_owned(),
            error,
        });
    }

    async fn attempt(&mut self, launch: &Launch, session: &Session, key: &ApiKey) -> Attempt {
        let mut attempt = Attempt::default();
        let Ok(search_path) = env::join_paths(&launch.search_dirs) else {
            attempt.failure = Some("Could not start Claude Code.".to_owned());
            return attempt;
        };
        let mut command = Command::new(&launch.executable);
        command
            .args(claude_code::arguments(&launch.model, session))
            .current_dir(&launch.workspace)
            .env("PATH", search_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        match key {
            ApiKey::Present(secret) => {
                command.env("ANTHROPIC_API_KEY", secret.expose());
            }
            _ => {
                command
                    .env_remove("ANTHROPIC_API_KEY")
                    .env_remove("ANTHROPIC_AUTH_TOKEN");
            }
        }
        let Ok(mut child) = command.spawn() else {
            attempt.failure = Some("Could not start Claude Code.".to_owned());
            return attempt;
        };
        attempt.spawned = true;
        if let Some(group) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
            self.registry.set_group(&self.agent_id, group);
        }
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            attempt.failure = Some("Could not start Claude Code.".to_owned());
            self.registry.kill(&self.agent_id);
            let _ = child.wait().await;
            self.registry.clear(&self.agent_id);
            return attempt;
        };
        let message = launch.text.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(message.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let stderr_task = tokio::spawn(last_stderr_line(stderr));

        let mut parser = StreamParser::default();
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();
        loop {
            match read_line(&mut reader, &mut buffer).await {
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
                    self.registry.kill(&self.agent_id);
                    break;
                }
                Err(_) => {
                    attempt.failure = Some("Could not read the output of Claude Code.".to_owned());
                    self.registry.kill(&self.agent_id);
                    break;
                }
            }
        }
        drop(reader);
        if timeout(EXIT_GRACE, child.wait()).await.is_err() {
            self.registry.kill(&self.agent_id);
            let _ = child.wait().await;
        }
        self.registry.clear(&self.agent_id);
        attempt.stderr_line = timeout(STDERR_GRACE, stderr_task)
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        attempt
    }
}

fn redact(text: &str, key: &ApiKey) -> String {
    match key {
        ApiKey::Present(secret) if !secret.expose().is_empty() => {
            text.replace(secret.expose(), "[redacted]")
        }
        _ => text.to_owned(),
    }
}

fn failure_of(attempt: &Attempt, key: &ApiKey) -> String {
    if let Some(message) = &attempt.failure {
        return message.clone();
    }
    match &attempt.result {
        Some((_, text)) if !text.trim().is_empty() => redact(text, key),
        Some(_) => "Claude Code reported an error.".to_owned(),
        None => attempt
            .stderr_line
            .as_deref()
            .map(|line| redact(line, key))
            .unwrap_or_else(|| "Claude Code exited unexpectedly.".to_owned()),
    }
}

pub async fn run(
    db: &Database,
    sink: &dyn EventSink,
    registry: &TurnRegistry,
    launch: Launch,
    key: ApiKey,
) {
    let stored_session = match &launch.session {
        Session::New(id) | Session::Resume(id) => id.clone(),
    };
    let mut runner = Runner {
        db,
        sink,
        registry,
        agent_id: launch.agent_id.clone(),
        turn_id: launch.turn_id.clone(),
        clock: launch.started_at,
        next_position: 0,
        texts: HashMap::new(),
        step_inputs: HashMap::new(),
        stored_session,
        save_failed: false,
    };
    runner.emit(TurnEvent::Started);
    if !launch.workspace.is_dir() {
        runner.finish(
            "failed",
            Some("The workspace folder is missing.".to_owned()),
        );
        return;
    }
    if matches!(key, ApiKey::Unreadable) {
        runner.finish(
            "failed",
            Some("Could not read the saved API key.".to_owned()),
        );
        return;
    }
    let mut attempt = runner.attempt(&launch, &launch.session, &key).await;
    if matches!(launch.session, Session::Resume(_))
        && attempt.spawned
        && !attempt.saw_init
        && !attempt.succeeded()
    {
        let fresh = uuid::Uuid::new_v4().to_string();
        let agent_id = runner.agent_id.clone();
        runner.save(|connection| store::set_session(connection, &agent_id, Some(&fresh)));
        runner.stored_session.clone_from(&fresh);
        runner.add_notice(RESUME_NOTICE);
        attempt = runner.attempt(&launch, &Session::New(fresh), &key).await;
    }
    if attempt.succeeded() {
        runner.finish("completed", None);
    } else {
        runner.finish("failed", Some(failure_of(&attempt, &key)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turns::store::{self as turn_store, ItemView};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    const BASIC: &str = include_str!("../../tests/fixtures/claude-stream-basic.jsonl");
    const TOOL: &str = include_str!("../../tests/fixtures/claude-stream-tool.jsonl");

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
        _data: tempfile::TempDir,
        bin: tempfile::TempDir,
        workspace: tempfile::TempDir,
        records: tempfile::TempDir,
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
                _data: data,
                bin: tempfile::tempdir().unwrap(),
                workspace,
                records: tempfile::tempdir().unwrap(),
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
            "n=$(ls \"{records}\" | grep -c '^args' || true)\nfor a in \"$@\"; do echo \"$a\"; done > \"{records}/args.$n\"\ncat > \"{records}/stdin.$n\"\nprintf '%s' \"$ANTHROPIC_API_KEY\" > \"{records}/key.$n\"\nprintf '%s' \"$ANTHROPIC_AUTH_TOKEN\" > \"{records}/token.$n\"\n{rest}"
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

    const FIXTURE_SESSION: &str = "fa881671-4c21-46dc-a71d-edec9e9f44fd";

    #[tokio::test]
    async fn a_first_turn_passes_a_new_session_id_and_the_message_arrives_on_stdin() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("Reply with the word pong", ApiKey::Absent).await;
        let args = args(&world, 0);
        assert_eq!(
            args[..7],
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--model",
                "haiku"
            ]
        );
        assert_eq!(args[7], "--session-id");
        assert_eq!(args.len(), 9);
        assert_eq!(args[8].len(), 36);
        assert_ne!(args[8], FIXTURE_SESSION);
        assert!(!args.iter().any(|arg| arg.contains("pong")));
        assert_eq!(read(&world, "stdin.0"), "Reply with the word pong");
        assert_eq!(world.session().as_deref(), Some(FIXTURE_SESSION));
        assert!(world.registry.groups().is_empty());
    }

    #[tokio::test]
    async fn a_later_turn_resumes_the_stored_session() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("one", ApiKey::Absent).await;
        world.send("two", ApiKey::Absent).await;
        let second = args(&world, 1);
        assert_eq!(second[7..], ["--resume", FIXTURE_SESSION]);
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
        world.send("hi", ApiKey::Absent).await;
        let args = args(&world, 0);
        assert!(!args.iter().any(|arg| arg == "--model"));
        assert!(args
            .iter()
            .all(|arg| !arg.contains("permission") && !arg.contains("dangerously")));
        assert!(args
            .iter()
            .all(|arg| !arg.to_lowercase().contains("allowedtools")));
    }

    #[tokio::test]
    async fn the_child_sees_the_saved_key_only_when_one_is_passed() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        env::set_var("ANTHROPIC_API_KEY", "sk-ant-from-the-parent");
        env::set_var("ANTHROPIC_AUTH_TOKEN", "token-from-the-parent");
        world
            .send(
                "one",
                ApiKey::Present(SecretString::new("sk-ant-saved-0000")),
            )
            .await;
        world.send("two", ApiKey::Absent).await;
        env::remove_var("ANTHROPIC_API_KEY");
        env::remove_var("ANTHROPIC_AUTH_TOKEN");
        assert_eq!(read(&world, "key.0"), "sk-ant-saved-0000");
        assert_eq!(read(&world, "key.1"), "");
        assert_eq!(read(&world, "token.1"), "");
    }

    #[tokio::test]
    async fn an_unreadable_key_fails_the_turn_and_starts_no_process() {
        let world = World::new();
        let fixture = world.fixture("basic.jsonl", BASIC);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world.send("hi", ApiKey::Unreadable).await;
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
            ApiKey::Absent,
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
        world.send("Reply with the word pong", ApiKey::Absent).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "completed");
        assert_eq!(turn.error, None);
        assert_eq!(turn.user_message, "Reply with the word pong");
        assert!(turn.ended_at.is_some());
        assert!(matches!(&turn.items[..], [ItemView::Text { text, .. }] if text == "pong"));
        assert_eq!(
            world.sink.kinds(),
            ["started", "text-delta", "text-set", "ended"]
        );
    }

    #[tokio::test]
    async fn the_tool_fixture_saves_a_done_step_before_the_answer() {
        let world = World::new();
        let fixture = world.fixture("tool.jsonl", TOOL);
        world.install(&recording(
            &world,
            &format!("cat \"{}\"", fixture.display()),
        ));
        world
            .send("List the files in your folder", ApiKey::Absent)
            .await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "completed");
        let [ItemView::Step(step), ItemView::Text { text, .. }] = &turn.items[..] else {
            panic!("unexpected items: {:?}", turn.items);
        };
        assert_eq!(step.kind, "Bash");
        assert!(step.title.starts_with("ls -la"));
        assert_eq!(step.status, "done");
        let detail = step.detail.as_deref().unwrap();
        assert!(detail.contains("\"command\"") && detail.contains("CLAUDE.md"));
        assert!(text.contains("CLAUDE.md"));
        let kinds = world.sink.kinds();
        assert_eq!(kinds.first().map(String::as_str), Some("started"));
        assert_eq!(kinds.last().map(String::as_str), Some("ended"));
        for kind in ["step-started", "step-ended", "text-delta", "text-set"] {
            assert!(kinds.iter().any(|k| k == kind), "{kind}");
        }
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
        world.send("hi", ApiKey::Absent).await;
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
        world.send("hi", ApiKey::Absent).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(turn.error.as_deref(), Some("boom: bad thing"));
        assert_eq!(world.sink.kinds().last().map(String::as_str), Some("ended"));
    }

    #[tokio::test]
    async fn exiting_silently_gives_the_generic_message_and_the_key_is_never_echoed() {
        let world = World::new();
        world.install(&recording(&world, "exit 1"));
        world.send("hi", ApiKey::Absent).await;
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
        world.send("hi", ApiKey::Absent).await;
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert!(turn.error.unwrap().contains("too long"));
        assert!(world.registry.groups().is_empty());
    }

    #[tokio::test]
    async fn a_resume_failure_starts_a_new_session_once_and_adds_a_notice() {
        let world = World::new();
        world
            .db
            .with_connection(|c| c.execute("UPDATE agents SET provider_session_id = 'old-id'", []))
            .unwrap();
        let fixture = world.fixture("basic.jsonl", BASIC);
        let body = recording(
            &world,
            &format!(
                "case \"$*\" in *--resume*) echo 'No conversation found with session ID: old-id' >&2; exit 1;; esac\ncat \"{}\"",
                fixture.display()
            ),
        );
        world.install(&body);
        world.send("hello again", ApiKey::Absent).await;
        let first = args(&world, 0);
        let second = args(&world, 1);
        assert_eq!(first[7..], ["--resume", "old-id"]);
        assert_eq!(second[7], "--session-id");
        assert_ne!(second[8], "old-id");
        assert_eq!(read(&world, "stdin.1"), "hello again");
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
    async fn a_resume_that_fails_again_is_not_retried_a_second_time() {
        let world = World::new();
        world
            .db
            .with_connection(|c| c.execute("UPDATE agents SET provider_session_id = 'old-id'", []))
            .unwrap();
        world.install(&recording(&world, "echo 'nope' >&2\nexit 1"));
        world.send("hi", ApiKey::Absent).await;
        assert!(has_record(&world, "args.1"));
        assert!(!has_record(&world, "args.2"));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert_eq!(turn.error.as_deref(), Some("nope"));
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
        world.send("hi", ApiKey::Absent).await;
        assert!(!has_record(&world, "args.1"));
        let turn = only_turn(&world);
        assert_eq!(turn.status, "failed");
        assert!(turn.items.is_empty());
        assert_eq!(world.session().as_deref(), Some("old-id"));
    }
}
