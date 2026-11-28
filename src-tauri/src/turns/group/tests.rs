use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::params;

use super::super::claude_code::{assert_isolated, Session};
use super::super::runner::{self, ApiKey};
use super::super::{
    prepare, prepare_group, resume_group, start_after, start_next_group, EventSink, Launch,
    Prepared, TurnEnvelope, TurnError, TurnRegistry,
};
use super::*;
use crate::db::Database;

const BASIC: &str = include_str!("../../../tests/fixtures/claude-stream-basic.jsonl");
const FIXTURE_SESSION: &str = "69dbf927-5f35-4e12-b4f6-07074d11d933";

#[derive(Default)]
struct Recorder(Mutex<Vec<TurnEnvelope>>);

impl EventSink for Recorder {
    fn emit(&self, envelope: &TurnEnvelope) {
        self.0.lock().unwrap().push(envelope.clone());
    }
}

struct Fixture {
    _data: tempfile::TempDir,
    bin: tempfile::TempDir,
    records: tempfile::TempDir,
    folder: tempfile::TempDir,
    other_folder: tempfile::TempDir,
    workspaces: tempfile::TempDir,
    db: Database,
    registry: TurnRegistry,
    sink: Recorder,
}

impl Fixture {
    fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let workspaces = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let other_folder = tempfile::tempdir().unwrap();
        let db = Database::open(data.path()).unwrap();
        for (id, name) in [("a", "Ada"), ("b", "Bo")] {
            let workspace = workspaces.path().join(name);
            fs::create_dir_all(&workspace).unwrap();
            db.with_connection(|c| {
                c.execute(
                    "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                     VALUES (?1, ?2, 'red', 'claude-code', 'haiku', ?3, 1, 1)",
                    params![id, name, workspace.to_string_lossy()],
                )
                .unwrap();
            });
        }
        fs::write(
            workspaces.path().join("Ada").join("CLAUDE.md"),
            "Ada rules\n",
        )
        .unwrap();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO groups VALUES ('g', 'Team', ?1, 'a', 1, 1)",
                [folder.path().to_string_lossy()],
            )
            .unwrap();
            c.execute(
                "INSERT INTO groups VALUES ('g2', 'Crew', ?1, 'a', 1, 1)",
                [other_folder.path().to_string_lossy()],
            )
            .unwrap();
            c.execute_batch(
                "INSERT INTO group_members VALUES ('g', 'a', 0, 1), ('g', 'b', 1, 1),
                                                  ('g2', 'a', 0, 1), ('g2', 'b', 1, 1);",
            )
            .unwrap();
        });
        let bin = tempfile::tempdir().unwrap();
        let claude = bin.path().join("claude");
        fs::write(&claude, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _data: data,
            bin,
            records: tempfile::tempdir().unwrap(),
            folder,
            other_folder,
            workspaces,
            db,
            registry: TurnRegistry::default(),
            sink: Recorder::default(),
        }
    }

    fn dirs(&self) -> Vec<PathBuf> {
        vec![self.bin.path().to_path_buf()]
    }

    fn send(&self, group: &str, text: &str) -> Result<Prepared, TurnError> {
        prepare_group(&self.db, &self.registry, &self.dirs(), group, text)
    }

    fn started(&self, group: &str, text: &str) -> Launch {
        match self.send(group, text).unwrap() {
            Prepared::Started(launch) => launch,
            Prepared::Queued { .. } => panic!("expected a started turn"),
        }
    }

    fn queued(&self, group: &str, text: &str) -> Option<Launch> {
        match self.send(group, text).unwrap() {
            Prepared::Queued { launch, .. } => launch,
            Prepared::Started(_) => panic!("expected a queued message"),
        }
    }

    fn chat(&self, agent: &str, text: &str) -> Prepared {
        prepare(&self.db, &self.registry, &self.dirs(), agent, text).unwrap()
    }

    fn end(&self, launch: &Launch, status: &str) {
        self.registry.clear(&launch.agent_id, &launch.turn_id);
        self.db
            .with_connection(|c| match &launch.group {
                Some(group) => super::finish_turn(
                    c,
                    &launch.turn_id,
                    &launch.agent_id,
                    &group.group_id,
                    status,
                    None,
                    50,
                ),
                None => store::finish_turn(c, &launch.turn_id, &launch.agent_id, status, None, 50),
            })
            .unwrap();
    }

    fn queue_texts(&self, group: &str) -> Vec<String> {
        self.db
            .with_connection(|c| list_queue(c, group))
            .unwrap()
            .into_iter()
            .map(|queued| queued.text)
            .collect()
    }

    fn value<T: rusqlite::types::FromSql>(&self, sql: &str, args: &[&str]) -> T {
        self.db.with_connection(|c| {
            c.query_row(sql, rusqlite::params_from_iter(args), |row| row.get(0))
                .unwrap()
        })
    }

    fn say(
        &self,
        agent: &str,
        group: &str,
        turn: &str,
        role: &str,
        author: Option<&str>,
        text: &str,
    ) {
        self.db.with_connection(|c| {
            c.execute(
                "INSERT INTO messages (id, agent_id, turn_id, group_id, author_agent_id, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 5)",
                params![uuid::Uuid::now_v7().to_string(), agent, turn, group, author, role, text],
            )
            .unwrap();
        });
    }

    fn bo_turn(&self, turn: &str, user: &str, reply: &str) {
        self.db.with_connection(|c| {
            c.execute(
                "INSERT INTO turns (id, agent_id, group_id, status, started_at)
                 VALUES (?1, 'b', 'g', 'completed', 5)",
                [turn],
            )
            .unwrap();
        });
        self.say("b", "g", turn, "user", None, user);
        self.say("b", "g", turn, "assistant", Some("b"), reply);
    }

    fn install(&self, body: &str) {
        let records = self.records.path().display();
        let fixture = self.records.path().join("basic.jsonl");
        fs::write(&fixture, BASIC).unwrap();
        let script = format!(
            "#!/bin/sh\nn=$(ls \"{records}\" | grep -c '^args' || true)\nfor a in \"$@\"; do printf '%s\\0' \"$a\"; done > \"{records}/args.$n\"\ncat > \"{records}/stdin.$n\"\npwd > \"{records}/pwd.$n\"\n{body}\ncat \"{}\"\n",
            fixture.display()
        );
        let claude = self.bin.path().join("claude");
        fs::write(&claude, script).unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    }

    async fn run(&self, launch: Launch) {
        let mut launch = launch;
        launch
            .search_dirs
            .extend(["/usr/bin".into(), "/bin".into()]);
        runner::run(
            &self.db,
            &self.sink,
            &self.registry,
            launch,
            ApiKey::Subscription,
        )
        .await;
    }

    fn record(&self, name: &str) -> String {
        fs::read_to_string(self.records.path().join(name)).unwrap()
    }

    fn args(&self, n: usize) -> Vec<String> {
        self.record(&format!("args.{n}"))
            .split('\0')
            .filter(|arg| !arg.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

fn path_of(text: &str) -> PathBuf {
    fs::canonicalize(Path::new(text.trim())).unwrap()
}

#[test]
fn a_user_message_goes_to_the_lead_and_runs_in_the_group_folder_with_the_group_session() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    assert_eq!(launch.agent_id, "a");
    assert_eq!(launch.workspace, fx.folder.path());
    assert_eq!(launch.text, "hello");
    assert_eq!(launch.group.as_ref().unwrap().group_id, "g");
    assert!(matches!(launch.session, Session::New(_)));
    assert_eq!(
        fx.value::<String>(
            "SELECT group_id FROM turns WHERE id = ?1",
            &[&launch.turn_id]
        ),
        "g"
    );
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(*) FROM messages WHERE group_id = 'g' AND author_agent_id IS NULL AND role = 'user' AND content = 'hello'",
            &[]
        ),
        1
    );
}

#[test]
fn validation_and_refusals_start_nothing() {
    let fx = Fixture::new();
    let code = |result: Result<Prepared, TurnError>| result.err().expect("an error").code;
    assert_eq!(code(fx.send("g", "  ")), "empty");
    assert_eq!(code(fx.send("missing", "hi")), "not-found");
    fx.db
        .with_connection(|c| c.execute("UPDATE groups SET lead_agent_id = NULL WHERE id = 'g'", []))
        .unwrap();
    assert_eq!(code(fx.send("g", "hi")), "no-lead");
    fx.db
        .with_connection(|c| c.execute("UPDATE groups SET lead_agent_id = 'a' WHERE id = 'g'", []))
        .unwrap();
    fx.db
        .with_connection(|c| {
            c.execute(
                "DELETE FROM group_members WHERE agent_id = 'b' AND group_id = 'g'",
                [],
            )
        })
        .unwrap();
    assert_eq!(code(fx.send("g", "hi")), "too-few-members");
    fs::remove_dir(fx.folder.path()).unwrap();
    assert_eq!(code(fx.send("g", "hi")), "folder-missing");
    fx.registry
        .closing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(code(fx.send("g2", "hi")), "shutting-down");
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM turns", &[]), 0);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM messages", &[]), 0);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM group_queue", &[]), 0);
}

#[test]
fn only_one_turn_runs_per_group_and_the_queue_advances_one_message_at_a_time() {
    let fx = Fixture::new();
    let first = fx.started("g", "one");
    assert!(fx.queued("g", "two").is_none());
    assert!(fx.queued("g", "three").is_none());
    assert_eq!(fx.queue_texts("g"), ["two", "three"]);
    assert!(start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").is_none());
    fx.end(&first, "completed");
    let second = start_after(&fx.db, &fx.registry, &fx.dirs(), "a", Some("g")).unwrap();
    assert_eq!(second.text, "two");
    assert_eq!(fx.queue_texts("g"), ["three"]);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM turns", &[]), 2);
    assert!(start_after(&fx.db, &fx.registry, &fx.dirs(), "a", Some("g")).is_none());
    assert_eq!(fx.queue_texts("g"), ["three"]);
}

#[test]
fn an_agent_busy_in_its_own_chat_delays_its_group_turn_until_that_turn_ends() {
    let fx = Fixture::new();
    let Prepared::Started(own) = fx.chat("a", "private work") else {
        panic!("expected a started turn");
    };
    assert!(fx.queued("g", "hello group").is_none());
    assert_eq!(fx.queue_texts("g"), ["hello group"]);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM turns", &[]), 1);
    assert!(start_after(&fx.db, &fx.registry, &fx.dirs(), "a", None).is_none());
    fx.end(&own, "completed");
    let launch = start_after(&fx.db, &fx.registry, &fx.dirs(), "a", None).unwrap();
    assert_eq!(launch.group.as_ref().unwrap().group_id, "g");
    assert_eq!(launch.text, "hello group");
    assert_eq!(launch.workspace, fx.folder.path());
    assert!(fx.queue_texts("g").is_empty());
}

#[test]
fn an_agent_busy_in_another_group_delays_its_turn_in_this_group() {
    let fx = Fixture::new();
    let elsewhere = fx.started("g2", "work in crew");
    assert!(fx.queued("g", "hello team").is_none());
    assert_eq!(fx.queue_texts("g"), ["hello team"]);
    assert_eq!(fx.queue_texts("g2"), Vec::<String>::new());
    fx.end(&elsewhere, "completed");
    let launch = start_after(&fx.db, &fx.registry, &fx.dirs(), "a", Some("g2")).unwrap();
    assert_eq!(launch.group.as_ref().unwrap().group_id, "g");
}

#[test]
fn a_one_to_one_message_waits_while_the_agent_works_in_a_group_and_starts_after_it() {
    let fx = Fixture::new();
    let group_turn = fx.started("g", "group work");
    let Prepared::Queued { launch, .. } = fx.chat("a", "private") else {
        panic!("expected a queued message");
    };
    assert!(launch.is_none());
    fx.end(&group_turn, "completed");
    let next = start_after(&fx.db, &fx.registry, &fx.dirs(), "a", Some("g")).unwrap();
    assert!(next.group.is_none());
    assert_eq!(next.text, "private");
    assert_eq!(
        next.workspace,
        Path::new(&fx.value::<String>("SELECT workspace_path FROM agents WHERE id = 'a'", &[]))
    );
}

#[test]
fn the_input_holds_exactly_the_unseen_messages_in_order_then_the_addressed_message() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    assert_eq!(first.text, "hello");
    fx.say(
        "a",
        "g",
        &first.turn_id,
        "assistant",
        Some("a"),
        "Hi, I am Ada",
    );
    fx.say(
        "a",
        "g",
        &first.turn_id,
        "notice",
        Some("a"),
        "a notice never shows",
    );
    fx.end(&first, "completed");
    fx.bo_turn("tb", "ask Bo", "Bo answers");
    fx.say("b", "g", "tb", "assistant", Some("b"), "");
    let second = fx.started("g", "what now");
    assert_eq!(second.text, "User: ask Bo\nBo: Bo answers\n\nwhat now");
    fx.end(&second, "completed");
    let third = fx.started("g", "again");
    assert_eq!(third.text, "again");
}

#[test]
fn messages_of_another_group_never_reach_the_input() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO turns (id, agent_id, group_id, status, started_at) VALUES ('tx', 'b', 'g2', 'completed', 5)",
            [],
        )
        .unwrap();
    });
    fx.say(
        "b",
        "g2",
        "tx",
        "assistant",
        Some("b"),
        "secret of the other group",
    );
    let launch = fx.started("g", "hello");
    assert_eq!(launch.text, "hello");
}

#[test]
fn last_seen_moves_to_the_addressed_message_at_the_start_and_to_the_agents_own_last_message_at_the_end(
) {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    let user_rowid: i64 = fx.value(
        "SELECT rowid FROM messages WHERE turn_id = ?1 AND role = 'user'",
        &[&launch.turn_id],
    );
    let seen = || {
        fx.value::<i64>(
            "SELECT last_seen_rowid FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
            &[],
        )
    };
    assert_eq!(seen(), user_rowid);
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(*) FROM agent_group_sessions WHERE agent_id = 'b'",
            &[]
        ),
        0
    );
}

#[test]
fn last_seen_at_the_end_reaches_the_agents_own_last_message() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    fx.say("a", "g", &launch.turn_id, "assistant", Some("a"), "one");
    fx.say("a", "g", &launch.turn_id, "assistant", Some("a"), "two");
    let last: i64 = fx.value("SELECT MAX(rowid) FROM messages", &[]);
    fx.end(&launch, "completed");
    assert_eq!(
        fx.value::<i64>(
            "SELECT last_seen_rowid FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
            &[]
        ),
        last
    );
}

#[test]
fn a_group_session_is_separate_from_the_one_to_one_session_and_from_other_groups() {
    let fx = Fixture::new();
    let Prepared::Started(own) = fx.chat("a", "private") else {
        panic!("expected a started turn");
    };
    let Session::New(private_id) = own.session.clone() else {
        panic!("expected a new session");
    };
    fx.end(&own, "completed");
    let group_launch = fx.started("g", "hello");
    let Session::New(group_id) = group_launch.session.clone() else {
        panic!("a group turn starts its own session");
    };
    assert_ne!(group_id, private_id);
    assert_eq!(
        fx.value::<String>("SELECT provider_session_id FROM agents WHERE id = 'a'", &[]),
        private_id
    );
    assert_eq!(
        fx.value::<String>(
            "SELECT provider_session_id FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
            &[]
        ),
        group_id
    );
    fx.end(&group_launch, "completed");
    let again = fx.started("g", "more");
    assert_eq!(again.session, Session::Resume(group_id.clone()));
    fx.end(&again, "completed");
    let elsewhere = fx.started("g2", "other group");
    let Session::New(other_id) = elsewhere.session else {
        panic!("another group starts its own session");
    };
    assert_ne!(other_id, group_id);
    assert_ne!(other_id, private_id);
}

#[test]
fn the_briefing_names_the_group_the_members_with_their_roles_and_the_hand_off_rule() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    let prompt = launch.group.as_ref().unwrap().system_prompt.clone();
    assert_eq!(
        prompt,
        "Ada rules\n\nYou are Ada, a member of the group chat \"Team\". Members: Ada (you, lead), Bo. The user and other members write in this chat. To hand a task to another member, start a line with @Name followed by the task."
    );
    fx.end(&launch, "completed");
    fx.db
        .with_connection(|c| c.execute("UPDATE groups SET lead_agent_id = 'b' WHERE id = 'g'", []))
        .unwrap();
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'b', 'for bo', 0, 1)",
            [],
        )
        .unwrap()
    });
    let bo = start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").unwrap();
    assert_eq!(
        bo.group.unwrap().system_prompt,
        "You are Bo, a member of the group chat \"Team\". Members: Ada, Bo (you, lead). The user and other members write in this chat. To hand a task to another member, start a line with @Name followed by the task."
    );
}

#[test]
fn an_unreadable_instruction_file_leaves_the_briefing_alone() {
    assert_eq!(system_prompt("  \n", "brief"), "brief");
    assert_eq!(system_prompt("rules\n\n", "brief"), "rules\n\nbrief");
    let fx = Fixture::new();
    fs::remove_file(fx.workspaces.path().join("Ada").join("CLAUDE.md")).unwrap();
    let launch = fx.started("g", "hello");
    assert!(launch
        .group
        .unwrap()
        .system_prompt
        .starts_with("You are Ada,"));
}

#[tokio::test]
async fn a_group_turn_runs_in_the_group_folder_isolated_with_the_prompt_and_the_input() {
    let fx = Fixture::new();
    fx.install("");
    let first = fx.started("g", "hello");
    fx.run(first).await;
    fx.bo_turn("tb", "ask Bo", "Bo answers");
    let second = fx.started("g", "what now");
    fx.run(second).await;
    let folder = fs::canonicalize(fx.folder.path()).unwrap();
    assert_eq!(path_of(&fx.record("pwd.0")), folder);
    assert_eq!(path_of(&fx.record("pwd.1")), folder);
    assert_eq!(fx.record("stdin.0"), "hello");
    assert_eq!(
        fx.record("stdin.1"),
        "User: ask Bo\nBo: Bo answers\n\nwhat now"
    );
    let args = fx.args(0);
    assert_isolated(&args);
    assert_eq!(
        args[args
            .iter()
            .position(|arg| arg == "--setting-sources")
            .unwrap()
            + 1],
        "project"
    );
    let at = args
        .iter()
        .position(|arg| arg == "--append-system-prompt")
        .unwrap();
    assert!(args[at + 1].starts_with("Ada rules"));
    assert!(args[at + 1].contains("You are Ada, a member of the group chat \"Team\"."));
    assert_eq!(args.len(), at + 2);
    assert!(args.contains(&"--session-id".to_owned()));
    let second_args = fx.args(1);
    assert_isolated(&second_args);
    let resume = second_args
        .iter()
        .position(|arg| arg == "--resume")
        .unwrap();
    assert_eq!(second_args[resume + 1], FIXTURE_SESSION);
}

#[tokio::test]
async fn a_group_turn_stores_its_session_and_messages_in_the_group_and_leaves_the_agent_row_alone()
{
    let fx = Fixture::new();
    fx.install("");
    let launch = fx.started("g", "hello");
    let turn_id = launch.turn_id.clone();
    fx.run(launch).await;
    assert_eq!(
        fx.value::<String>(
            "SELECT provider_session_id FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
            &[]
        ),
        FIXTURE_SESSION
    );
    assert_eq!(
        fx.value::<Option<String>>("SELECT provider_session_id FROM agents WHERE id = 'a'", &[]),
        None
    );
    assert_eq!(
        fx.value::<String>("SELECT status FROM turns WHERE id = ?1", &[&turn_id]),
        "completed"
    );
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(*) FROM messages WHERE turn_id = ?1 AND role = 'assistant' AND group_id = 'g' AND author_agent_id = 'a' AND content = 'pong'",
            &[&turn_id]
        ),
        1
    );
    let events = fx.sink.0.lock().unwrap();
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .all(|event| event.group_id.as_deref() == Some("g")));
    drop(events);
    let page = fx
        .db
        .with_connection(|c| store::group_conversation(c, "g", None))
        .unwrap();
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].agent_id, "a");
    assert_eq!(page.turns[0].user_message, "hello");
    let own = fx
        .db
        .with_connection(|c| store::conversation(c, "a", None))
        .unwrap();
    assert!(own.turns.is_empty());
    let last_message: i64 = fx.value("SELECT MAX(rowid) FROM messages", &[]);
    assert_eq!(
        fx.value::<i64>(
            "SELECT last_seen_rowid FROM agent_group_sessions WHERE agent_id = 'a'",
            &[]
        ),
        last_message
    );
}

#[tokio::test]
async fn a_missing_group_folder_fails_the_turn_with_the_group_wording() {
    let fx = Fixture::new();
    fx.install("");
    let mut launch = fx.started("g", "hello");
    launch.workspace = fx.folder.path().join("gone");
    let turn_id = launch.turn_id.clone();
    fx.run(launch).await;
    assert_eq!(
        fx.value::<String>("SELECT error FROM turns WHERE id = ?1", &[&turn_id]),
        "The group folder is missing."
    );
}

#[test]
fn a_group_queue_message_can_be_edited_and_removed_until_it_starts() {
    let fx = Fixture::new();
    fx.started("g", "one");
    fx.queued("g", "two");
    fx.queued("g", "three");
    let queue = fx.db.with_connection(|c| list_queue(c, "g")).unwrap();
    let (group, edited) = fx
        .db
        .with_connection(|c| edit_queued(c, &queue[0].id, "two, edited"))
        .unwrap()
        .unwrap();
    assert_eq!((group.as_str(), edited.text.as_str()), ("g", "two, edited"));
    let removed = fx
        .db
        .with_connection(|c| remove_queued(c, &queue[1].id))
        .unwrap();
    assert_eq!(removed.as_deref(), Some("g"));
    assert_eq!(fx.queue_texts("g"), ["two, edited"]);
    assert!(fx
        .db
        .with_connection(|c| remove_queued(c, &queue[1].id))
        .unwrap()
        .is_none());
    assert!(fx
        .db
        .with_connection(|c| edit_queued(c, "missing", "x"))
        .unwrap()
        .is_none());
}

#[test]
fn an_interrupted_group_turn_leaves_the_queue_waiting_for_resume_and_resume_starts_it() {
    let fx = Fixture::new();
    fx.started("g", "one");
    fx.queued("g", "two");
    fx.queued("g", "three");
    assert_eq!(super::super::sweep_interrupted(&fx.db).unwrap(), 1);
    assert_eq!(fx.queue_texts("g"), ["two", "three"]);
    assert_eq!(
        fx.value::<String>("SELECT status FROM turns", &[]),
        "interrupted"
    );
    let launch = resume_group(&fx.db, &fx.registry, &fx.dirs(), "g")
        .unwrap()
        .unwrap();
    assert_eq!(launch.text, "two");
    assert_eq!(fx.queue_texts("g"), ["three"]);
    assert!(resume_group(&fx.db, &fx.registry, &fx.dirs(), "g")
        .unwrap()
        .is_none());
    let empty = resume_group(&fx.db, &fx.registry, &fx.dirs(), "g2").unwrap();
    assert!(empty.is_none());
}

#[test]
fn resume_is_refused_while_closing() {
    let fx = Fixture::new();
    fx.started("g", "one");
    fx.queued("g", "two");
    fx.registry
        .closing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let error = resume_group(&fx.db, &fx.registry, &fx.dirs(), "g")
        .err()
        .unwrap();
    assert_eq!(error.code, "shutting-down");
    assert_eq!(fx.queue_texts("g"), ["two"]);
}

#[test]
fn a_stop_finds_the_agent_of_a_running_group_turn_only_in_its_own_group() {
    let fx = Fixture::new();
    let launch = fx.started("g", "one");
    let found = |group: &str| {
        fx.db
            .with_connection(|c| running_turn_agent(c, group, &launch.turn_id))
            .unwrap()
    };
    assert_eq!(found("g").as_deref(), Some("a"));
    assert_eq!(found("g2"), None);
}

#[test]
fn a_provider_that_is_missing_leaves_the_message_unsent() {
    let fx = Fixture::new();
    fs::remove_file(fx.bin.path().join("claude")).unwrap();
    let error = fx.send("g", "hello").err().unwrap();
    assert_eq!(error.code, "provider-unavailable");
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM messages", &[]), 0);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM group_queue", &[]), 0);
    let _ = &fx.other_folder;
}

#[test]
fn the_agents_own_partial_messages_of_an_interrupted_turn_are_not_replayed_to_it() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    fx.say(
        "a",
        "g",
        &launch.turn_id,
        "assistant",
        Some("a"),
        "half an ans",
    );
    fx.registry.clear(&launch.agent_id, &launch.turn_id);
    assert_eq!(super::super::sweep_interrupted(&fx.db).unwrap(), 1);
    let next = fx.started("g", "again");
    assert_eq!(next.text, "again");
}

#[test]
fn a_member_queued_behind_a_running_turn_of_the_group_does_not_start_beside_it() {
    let fx = Fixture::new();
    fx.started("g", "lead works");
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'b', 'for bo', 0, 1)",
            [],
        )
        .unwrap()
    });
    assert!(start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").is_none());
    assert_eq!(fx.queue_texts("g"), ["for bo"]);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM turns", &[]), 1);
}
