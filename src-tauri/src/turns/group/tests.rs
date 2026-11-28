use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::params;

use super::super::claude_code::{assert_isolated, Session};
use super::super::runner::{self, ApiKey, RunEnd};
use super::super::{
    note_end, prepare, prepare_group, resume, resume_group, start_after, start_next_group, Chain,
    EventSink, Finished, Launch, Prepared, TurnEnvelope, TurnError, TurnRegistry,
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
        let data_path = data.path().to_path_buf();
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
            registry: TurnRegistry::with_temp_dir(data_path.join("tmp")),
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

    fn after(&self, agent: &str, group: Option<&str>, end: RunEnd) -> Vec<Launch> {
        self.after_in(agent, group, end, &Chain::default())
    }

    fn failed(&self, agent: &str, group: Option<&str>, end: RunEnd) -> Vec<Launch> {
        let mut chain = Chain::default();
        chain.record(agent, group, end);
        self.after_in(agent, group, end, &chain)
    }

    fn after_in(
        &self,
        agent: &str,
        group: Option<&str>,
        end: RunEnd,
        chain: &Chain,
    ) -> Vec<Launch> {
        start_after(
            &self.db,
            &self.registry,
            &self.dirs(),
            &Finished {
                agent_id: agent,
                group_id: group,
                end,
                chain,
            },
        )
    }

    fn after_one(&self, agent: &str, group: Option<&str>) -> Option<Launch> {
        let mut launches = self.after(agent, group, RunEnd::Finished);
        assert!(launches.len() <= 1);
        launches.pop()
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
                    &super::Finish {
                        turn_id: &launch.turn_id,
                        agent_id: &launch.agent_id,
                        group_id: &group.group_id,
                        status,
                        error: None,
                        restore_seen: None,
                        clear_context_hash: false,
                        now: 50,
                    },
                )
                .map(|_| ()),
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
            "#!/bin/sh\ncase \"$1\" in --version) echo '2.1.332 (Claude Code)'; exit 0;; esac\ncase \"$1\" in --help) [ -f \"{records}/help-delay\" ] && sleep 2; echo '  --system-prompt-snapshot <on|off>'; exit 0;; esac\nn=$(ls \"{records}\" | grep -c '^args' || true)\nfor a in \"$@\"; do printf '%s\\0' \"$a\"; done > \"{records}/args.$n\"\nprev=\"\"; for a in \"$@\"; do if [ \"$prev\" = --append-system-prompt-file ]; then cp \"$a\" \"{records}/prompt.$n\"; stat -f %Lp \"$a\" > \"{records}/mode.$n\"; echo \"$a\" > \"{records}/promptpath.$n\"; fi; prev=\"$a\"; done\nprev=\"\"; for a in \"$@\"; do if [ \"$prev\" = --settings ]; then cp \"$a\" \"{records}/settings.$n\"; fi; prev=\"$a\"; done\ncat > \"{records}/stdin.$n\"\npwd > \"{records}/pwd.$n\"\n{body}\ncat \"{}\"\n",
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
        let record = self.record(&format!("args.{n}"));
        record
            .strip_suffix('\0')
            .unwrap_or(&record)
            .split('\0')
            .map(str::to_owned)
            .collect()
    }
}

fn input(launch: &Launch) -> &str {
    launch
        .text
        .split_once(super::MESSAGES_HEADER)
        .expect("a group turn input has a messages part")
        .1
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
    assert_eq!(input(&launch), "hello");
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
    let second = fx.after_one("a", Some("g")).unwrap();
    assert_eq!(input(&second), "two");
    assert_eq!(fx.queue_texts("g"), ["three"]);
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM turns", &[]), 2);
    assert!(fx.after_one("a", Some("g")).is_none());
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
    assert!(fx.after_one("a", None).is_none());
    fx.end(&own, "completed");
    let launch = fx.after_one("a", None).unwrap();
    assert_eq!(launch.group.as_ref().unwrap().group_id, "g");
    assert_eq!(input(&launch), "hello group");
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
    let launch = fx.after_one("a", Some("g2")).unwrap();
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
    let next = fx.after_one("a", Some("g")).unwrap();
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
    assert_eq!(input(&first), "hello");
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
    assert_eq!(input(&second), "User: ask Bo\nBo: Bo answers\n\nwhat now");
    fx.end(&second, "completed");
    let third = fx.started("g", "again");
    assert_eq!(input(&third), "again");
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
    assert_eq!(input(&launch), "hello");
}

#[test]
fn last_seen_moves_to_the_addressed_message_when_the_turn_starts() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    let user_seq: i64 = fx.value(
        "SELECT seq FROM messages WHERE turn_id = ?1 AND role = 'user'",
        &[&launch.turn_id],
    );
    let seen = || {
        fx.value::<i64>(
            "SELECT last_seen_seq FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
            &[],
        )
    };
    assert_eq!(seen(), user_seq);
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
    let last: i64 = fx.value("SELECT MAX(seq) FROM messages", &[]);
    fx.end(&launch, "completed");
    assert_eq!(
        fx.value::<i64>(
            "SELECT last_seen_seq FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
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

const HAND_OFF: &str = "The user and other members write in this chat. To hand a task to another member, start a line with @Name followed by the task.";

#[test]
fn the_turn_input_opens_with_the_briefing_and_the_folder_instructions_before_the_messages() {
    let fx = Fixture::new();
    fs::write(
        fx.folder.path().join("CLAUDE.md"),
        "Use tabs.\nPROBE-LINE: zebra\n",
    )
    .unwrap();
    let launch = fx.started("g", "hello");
    assert_eq!(
        launch.text,
        format!(
            "You are Ada, a member of the group chat \"Team\". Members: Ada (you, lead), Bo. {HAND_OFF}\n\nInstructions from the group folder's CLAUDE.md:\nUse tabs.\nPROBE-LINE: zebra\n\n{}hello",
            super::MESSAGES_HEADER
        )
    );
    let group = launch.group.as_ref().unwrap();
    assert_eq!(group.instructions, "Ada rules\n");
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
    assert!(bo.text.starts_with(&format!(
        "You are Bo, a member of the group chat \"Team\". Members: Ada, Bo (you, lead). {HAND_OFF}\n\nInstructions from the group folder's CLAUDE.md:"
    )));
    assert_eq!(bo.group.unwrap().instructions, "");
}

#[test]
fn a_missing_oversized_or_linked_folder_instructions_file_is_left_out() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    assert!(!launch.text.contains("Instructions from the group folder"));
    fx.end(&launch, "completed");
    fs::write(
        fx.folder.path().join("CLAUDE.md"),
        "x".repeat(super::FOLDER_INSTRUCTIONS_MAX_BYTES as usize + 1),
    )
    .unwrap();
    let big = fx.started("g", "again");
    assert!(!big.text.contains("Instructions from the group folder"));
    assert_eq!(
        big.group.as_ref().unwrap().folder_notice.as_deref(),
        Some("The group folder's CLAUDE.md was not loaded (it is larger than 64 KiB).")
    );
    fx.end(&big, "completed");
    fs::write(fx.folder.path().join("CLAUDE.md"), [0xff, 0xfe, 0x00]).unwrap();
    let binary = fx.started("g", "binary");
    assert_eq!(
        binary.group.as_ref().unwrap().folder_notice.as_deref(),
        Some("The group folder's CLAUDE.md was not loaded (it is not UTF-8 text).")
    );
    fx.end(&binary, "completed");
    let secret = fx.workspaces.path().join("secret.txt");
    fs::write(&secret, "do not leak").unwrap();
    fs::remove_file(fx.folder.path().join("CLAUDE.md")).unwrap();
    std::os::unix::fs::symlink(&secret, fx.folder.path().join("CLAUDE.md")).unwrap();
    let linked = fx.started("g", "third");
    assert_eq!(
        linked.group.as_ref().unwrap().folder_notice.as_deref(),
        Some("The group folder's CLAUDE.md was not loaded (it is not a regular file).")
    );
    assert!(!linked.text.contains("do not leak"));
    assert!(!linked.text.contains("Instructions from the group folder"));
    fx.end(&linked, "completed");
    fs::remove_file(fx.folder.path().join("CLAUDE.md")).unwrap();
    fs::write(&secret, "hard linked text").unwrap();
    fs::hard_link(&secret, fx.folder.path().join("CLAUDE.md")).unwrap();
    let hard = fx.started("g", "fourth");
    assert_eq!(
        hard.group.as_ref().unwrap().folder_notice.as_deref(),
        Some("The group folder's CLAUDE.md was not loaded (it has more than one hard link).")
    );
    assert!(!hard.text.contains("hard linked text"));
}

#[test]
fn a_hard_linked_member_instruction_file_is_left_out_of_a_group_turn() {
    let fx = Fixture::new();
    let own = fx.workspaces.path().join("Ada").join("CLAUDE.md");
    let outside = fx.workspaces.path().join("outside.md");
    fs::write(&outside, "linked rules\n").unwrap();
    fs::remove_file(&own).unwrap();
    fs::hard_link(&outside, &own).unwrap();
    let launch = fx.started("g", "hello");
    let group = launch.group.as_ref().unwrap();
    assert_eq!(group.instructions, "");
    assert_eq!(
        group.member_notice.as_deref(),
        Some("The workspace's CLAUDE.md was not loaded (it has more than one hard link).")
    );
}

#[test]
fn a_usable_member_instruction_file_gives_no_notice() {
    let fx = Fixture::new();
    let launch = fx.started("g", "hello");
    let group = launch.group.as_ref().unwrap();
    assert_eq!(group.instructions, "Ada rules\n");
    assert_eq!(group.member_notice, None);
}

#[tokio::test]
async fn a_group_turn_is_confined_to_the_group_folder_and_protects_every_member_workspace() {
    let fx = Fixture::new();
    fx.install("");
    let launch = fx.started("g", "hello");
    fx.run(launch).await;
    let settings: serde_json::Value = serde_json::from_str(&fx.record("settings.0")).unwrap();
    let folder = fs::canonicalize(fx.folder.path()).unwrap();
    let folder = folder.to_string_lossy().into_owned();
    let write = settings["sandbox"]["filesystem"]["allowWrite"]
        .as_array()
        .unwrap();
    assert_eq!(write[0].as_str(), Some(folder.as_str()));
    let read: Vec<&str> = settings["sandbox"]["filesystem"]["denyRead"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect();
    assert!(!read.contains(&folder.as_str()));
    let data = fs::canonicalize(fx._data.path()).unwrap();
    assert!(read.contains(&data.to_string_lossy().as_ref()));
    for name in ["Ada", "Bo"] {
        let workspace = fs::canonicalize(fx.workspaces.path().join(name)).unwrap();
        assert!(
            read.contains(&workspace.to_string_lossy().as_ref()),
            "{name}"
        );
    }
    assert_eq!(settings["sandbox"]["failIfUnavailable"], true);
    assert_eq!(
        settings["permissions"]["allow"][1],
        format!("Edit(/{folder}/**)")
    );
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
    let stdin = |n: usize| {
        fx.record(&format!("stdin.{n}"))
            .split_once(super::MESSAGES_HEADER)
            .unwrap()
            .1
            .to_owned()
    };
    assert_eq!(stdin(0), "hello");
    assert_eq!(stdin(1), "User: ask Bo\nBo: Bo answers\n\nwhat now");
    assert!(fx
        .record("stdin.0")
        .starts_with("You are Ada, a member of the group chat \"Team\"."));
    let args = fx.args(0);
    assert_isolated(&args);
    let sources = args
        .iter()
        .position(|arg| arg == "--setting-sources")
        .unwrap();
    assert_eq!(args[sources + 1], "");
    let file = args
        .iter()
        .position(|arg| arg == "--append-system-prompt-file")
        .unwrap();
    assert_eq!(fx.record("prompt.0"), "Ada rules\n");
    assert_eq!(fx.record("mode.0").trim(), "600");
    let path = PathBuf::from(fx.record("promptpath.0").trim());
    assert_eq!(args[file + 1], path.to_string_lossy());
    assert!(path.starts_with(fx._data.path().join("tmp")));
    assert!(!path.exists());
    assert_eq!(
        fs::read_dir(fx._data.path().join("tmp")).unwrap().count(),
        0
    );
    assert!(!args.iter().any(|arg| arg == "--append-system-prompt"));
    assert_eq!(
        args[args.len() - 4..args.len() - 2],
        ["--system-prompt-snapshot", "off"]
    );
    assert_eq!(args[args.len() - 2], "--settings");
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
    assert_eq!(page.turns[0].agent_id.as_deref(), Some("a"));
    assert_eq!(page.turns[0].user_message, "hello");
    let own = fx
        .db
        .with_connection(|c| store::conversation(c, "a", None))
        .unwrap();
    assert!(own.turns.is_empty());
    let last_message: i64 = fx.value("SELECT MAX(seq) FROM messages", &[]);
    assert_eq!(
        fx.value::<i64>(
            "SELECT last_seen_seq FROM agent_group_sessions WHERE agent_id = 'a'",
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
        .with_connection(|c| remove_queued(c, &queue[1].id, 60))
        .unwrap();
    assert_eq!(removed.as_deref(), Some("g"));
    assert_eq!(fx.queue_texts("g"), ["two, edited"]);
    assert!(fx
        .db
        .with_connection(|c| remove_queued(c, &queue[1].id, 60))
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
    assert_eq!(input(&launch), "two");
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
    assert_eq!(input(&next), "again");
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

#[tokio::test]
async fn a_group_folder_with_project_settings_loads_none_of_them_and_its_instructions_reach_the_turn(
) {
    let fx = Fixture::new();
    fx.install("");
    fs::create_dir(fx.folder.path().join(".claude")).unwrap();
    fs::write(
        fx.folder.path().join(".claude").join("settings.json"),
        r#"{"hooks":{"PreToolUse":[{"matcher":"","hooks":[{"type":"command","command":"touch hooked"}]}]},"apiKeyHelper":"echo key"}"#,
    )
    .unwrap();
    fs::write(
        fx.folder.path().join("CLAUDE.md"),
        "PROBE-LINE: zebra-quartz-4471\n",
    )
    .unwrap();
    let launch = fx.started("g", "hello");
    fx.run(launch).await;
    let args = fx.args(0);
    assert_isolated(&args);
    let at = args
        .iter()
        .position(|arg| arg == "--setting-sources")
        .unwrap();
    assert_eq!(args[at + 1], "");
    assert!(!args.iter().any(|arg| arg == "project"));
    let stdin = fx.record("stdin.0");
    assert!(stdin.contains("PROBE-LINE: zebra-quartz-4471"));
    assert!(stdin.ends_with("hello"));
}

#[tokio::test]
async fn a_turn_that_fails_before_the_cli_read_its_input_restores_the_watermark() {
    for failure in ["folder", "spawn", "key"] {
        let fx = Fixture::new();
        fx.install("");
        let first = fx.started("g", "hello");
        fx.run(first).await;
        fx.bo_turn("tb", "ask Bo", "Bo answers");
        let seen = || {
            fx.value::<i64>(
                "SELECT last_seen_seq FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
                &[],
            )
        };
        let before = seen();
        let mut launch = fx.started("g", "will fail");
        assert!(seen() > before);
        let mut key = ApiKey::Subscription;
        match failure {
            "folder" => launch.workspace = fx.folder.path().join("gone"),
            "spawn" => launch.executable = fx.bin.path().join("missing"),
            _ => key = ApiKey::Missing,
        }
        launch
            .search_dirs
            .extend(["/usr/bin".into(), "/bin".into()]);
        runner::run(&fx.db, &fx.sink, &fx.registry, launch, key).await;
        assert_eq!(
            fx.value::<String>("SELECT status FROM turns ORDER BY rowid DESC LIMIT 1", &[]),
            "failed",
            "{failure}"
        );
        assert_eq!(seen(), before, "{failure}");
        let retry = fx.started("g", "again");
        assert_eq!(
            input(&retry),
            "User: ask Bo\nBo: Bo answers\nUser: will fail\n\nagain",
            "{failure}"
        );
    }
}

#[tokio::test]
async fn a_turn_that_got_as_far_as_the_session_start_keeps_the_watermark_when_it_fails_later() {
    let fx = Fixture::new();
    fx.install(&format!(
        "head -n 1 \"{}/basic.jsonl\"; exit 3",
        fx.records.path().display()
    ));
    let launch = fx.started("g", "hello");
    let turn_id = launch.turn_id.clone();
    fx.run(launch).await;
    assert_eq!(
        fx.value::<String>("SELECT status FROM turns WHERE id = ?1", &[&turn_id]),
        "failed"
    );
    let retry = fx.started("g", "again");
    assert_eq!(input(&retry), "again");
}

#[test]
fn when_a_turn_ends_the_group_and_the_freed_agent_each_start_what_can_start() {
    let fx = Fixture::new();
    let running = fx.started("g", "lead works");
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'b', 'for bo', 0, 1)",
            [],
        )
        .unwrap()
    });
    let Prepared::Queued { launch: None, .. } = fx.chat("a", "own work") else {
        panic!("the busy lead queues its own work");
    };
    fx.end(&running, "completed");
    let launches = fx.after("a", Some("g"), RunEnd::Finished);
    let mut started: Vec<(String, bool)> = launches
        .iter()
        .map(|launch| (launch.agent_id.clone(), launch.group.is_some()))
        .collect();
    started.sort();
    assert_eq!(started, [("a".to_owned(), false), ("b".to_owned(), true)]);
    assert!(fx.queue_texts("g").is_empty());
}

#[test]
fn a_group_entry_blocked_on_another_agent_does_not_hold_back_the_freed_agents_own_work() {
    let fx = Fixture::new();
    let Prepared::Started(_ada_busy) = fx.chat("a", "ada busy") else {
        panic!("expected a started turn");
    };
    fx.db
        .with_connection(|c| c.execute("UPDATE groups SET lead_agent_id = 'b' WHERE id = 'g2'", []))
        .unwrap();
    let bo_turn = fx.started("g2", "bo works");
    let Prepared::Queued { launch: None, .. } = fx.chat("b", "bo own") else {
        panic!("a busy agent queues");
    };
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'a', 'for ada', 0, 1)",
            [],
        )
        .unwrap()
    });
    fx.end(&bo_turn, "completed");
    let launches = fx.after("b", Some("g2"), RunEnd::Finished);
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].agent_id, "b");
    assert!(launches[0].group.is_none());
    assert_eq!(fx.queue_texts("g"), ["for ada"]);
}

#[test]
fn a_failure_before_the_spawn_does_not_freeze_the_other_chats_waiting_on_the_agent() {
    let fx = Fixture::new();
    let failing = fx.started("g", "will fail");
    let Prepared::Queued { launch: None, .. } = fx.chat("a", "own work") else {
        panic!("a busy agent queues");
    };
    fx.queued("g", "next in g");
    fx.end(&failing, "failed");
    let launches = fx.failed("a", Some("g"), RunEnd::ChatFailed);
    assert_eq!(launches.len(), 1);
    assert!(launches[0].group.is_none());
    assert_eq!(input_or_text(&launches[0]), "own work");
    assert_eq!(fx.queue_texts("g"), ["next in g"]);
}

#[test]
fn a_failed_own_chat_still_lets_a_waiting_group_message_start() {
    let fx = Fixture::new();
    let Prepared::Started(own) = fx.chat("a", "private") else {
        panic!("expected a started turn");
    };
    fx.queued("g", "waiting in g");
    fx.chat("a", "next private");
    fx.end(&own, "failed");
    let launches = fx.failed("a", None, RunEnd::ChatFailed);
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].group.as_ref().unwrap().group_id, "g");
    let queued: i64 = fx.value("SELECT COUNT(*) FROM queued_messages", &[]);
    assert_eq!(queued, 1);
}

fn input_or_text(launch: &Launch) -> &str {
    if launch.group.is_some() {
        input(launch)
    } else {
        &launch.text
    }
}

#[tokio::test]
async fn deleting_an_agent_keeps_its_group_history_and_removes_its_private_chat() {
    let fx = Fixture::new();
    fx.install("");
    let Prepared::Started(private) = fx.chat("a", "private") else {
        panic!("expected a started turn");
    };
    fx.run(private).await;
    let launch = fx.started("g", "hello");
    fx.run(launch).await;
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO queued_messages (id, agent_id, content, position, created_at)
             VALUES ('qp', 'a', 'queued private', 0, 1)",
            [],
        )
        .unwrap()
    });
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('qa', 'g', 'a', 'for ada', 0, 1), ('qb', 'g', 'b', 'for bo', 1, 1);",
        )
        .unwrap()
    });
    crate::agents::remove::delete(&fx.db, &Err(String::new()), "a", false, &|_| Ok(())).unwrap();
    assert_eq!(
        fx.value::<i64>("SELECT COUNT(*) FROM turns WHERE group_id IS NULL", &[]),
        0
    );
    assert_eq!(
        fx.value::<i64>("SELECT COUNT(*) FROM queued_messages", &[]),
        0
    );
    assert_eq!(
        fx.value::<i64>("SELECT COUNT(*) FROM messages WHERE group_id IS NULL", &[]),
        0
    );
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(*) FROM turns WHERE group_id = 'g' AND agent_id IS NULL",
            &[]
        ),
        1
    );
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(*) FROM messages WHERE group_id = 'g' AND agent_id IS NULL AND author_agent_id IS NULL",
            &[]
        ),
        fx.value::<i64>("SELECT COUNT(*) FROM messages WHERE group_id = 'g'", &[])
    );
    assert_eq!(
        fx.value::<String>(
            "SELECT content FROM messages WHERE role = 'assistant' AND group_id = 'g'",
            &[]
        ),
        "pong"
    );
    let page = fx
        .db
        .with_connection(|c| store::group_conversation(c, "g", None))
        .unwrap();
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].agent_id, None);
    assert_eq!(page.turns[0].user_message, "hello");
    assert_eq!(
        fx.value::<String>(
            "SELECT addressed_agent_id FROM group_queue WHERE id = 'qa'",
            &[]
        ),
        "b"
    );
    assert_eq!(fx.queue_texts("g"), ["for ada", "for bo"]);
    crate::agents::remove::delete(&fx.db, &Err(String::new()), "b", false, &|_| Ok(())).unwrap();
    assert!(fx.queue_texts("g").is_empty());
    assert_eq!(
        fx.value::<i64>("SELECT COUNT(*) FROM turns WHERE group_id = 'g'", &[]),
        1
    );
}

fn fill_queues(fx: &Fixture) {
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO queued_messages (id, agent_id, content, position, created_at)
             VALUES ('q1', 'a', 'private one', 0, 1), ('q2', 'a', 'private two', 1, 1);
             INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('g1', 'g', 'a', 'group one', 0, 1), ('g2', 'g', 'a', 'group two', 1, 1),
                    ('h1', 'g2', 'a', 'crew one', 0, 1);",
        )
        .unwrap()
    });
}

fn started_count(fx: &Fixture) -> i64 {
    fx.value("SELECT COUNT(*) FROM turns", &[])
}

#[test]
fn a_failure_that_belongs_to_the_agent_starts_nothing_addressed_to_it_in_any_chat() {
    for group in [None, Some("g")] {
        let fx = Fixture::new();
        fill_queues(&fx);
        fx.registry.hold("a");
        let launches = fx.after("a", group, RunEnd::AgentFailed);
        assert!(launches.is_empty(), "{group:?}");
        assert_eq!(started_count(&fx), 0);
        assert_eq!(fx.queue_texts("g"), ["group one", "group two"]);
        assert_eq!(fx.queue_texts("g2"), ["crew one"]);
        assert_eq!(
            fx.value::<i64>("SELECT COUNT(*) FROM queued_messages", &[]),
            2
        );
    }
}

#[test]
fn an_agent_failure_in_a_group_still_advances_the_group_when_its_head_is_another_agent() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'b', 'for bo', 0, 1), ('r', 'g', 'a', 'for ada', 1, 1)",
            [],
        )
        .unwrap()
    });
    fx.registry.hold("a");
    let launches = fx.after("a", Some("g"), RunEnd::AgentFailed);
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].agent_id, "b");
    fx.end(&launches[0], "completed");
    let next = fx.after("b", Some("g"), RunEnd::Finished);
    assert!(next.is_empty(), "the failed agent's entry must not start");
    assert_eq!(fx.queue_texts("g"), ["for ada"]);
}

#[test]
fn a_chat_failure_lets_the_other_chats_run_but_never_returns_to_a_failed_chat() {
    let fx = Fixture::new();
    fill_queues(&fx);
    let mut chain = Chain::default();
    chain.record("a", Some("g"), RunEnd::ChatFailed);
    let hop_two = fx.after_in("a", Some("g"), RunEnd::ChatFailed, &chain);
    let kinds: Vec<(Option<String>, String)> = hop_two
        .iter()
        .map(|launch| {
            (
                launch.group.as_ref().map(|group| group.group_id.clone()),
                launch.agent_id.clone(),
            )
        })
        .collect();
    assert_eq!(kinds, [(None, "a".to_owned())]);
    assert_eq!(fx.queue_texts("g"), ["group one", "group two"]);
    chain.record("a", None, RunEnd::ChatFailed);
    fx.end(&hop_two[0], "failed");
    let hop_three = fx.after_in("a", None, RunEnd::ChatFailed, &chain);
    let groups: Vec<String> = hop_three
        .iter()
        .filter_map(|launch| launch.group.as_ref().map(|group| group.group_id.clone()))
        .collect();
    assert_eq!(groups, ["g2"]);
    assert_eq!(fx.queue_texts("g"), ["group one", "group two"]);
    assert_eq!(
        fx.value::<i64>("SELECT COUNT(*) FROM queued_messages", &[]),
        1
    );
}

fn hash_of(fx: &Fixture) -> Option<String> {
    fx.value(
        "SELECT context_hash FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
        &[],
    )
}

#[tokio::test]
async fn an_unchanged_context_is_sent_once_and_left_out_of_later_turns() {
    let fx = Fixture::new();
    fx.install("");
    let first = fx.started("g", "hello");
    assert!(first.text.starts_with("You are Ada"));
    fx.run(first).await;
    assert!(hash_of(&fx).is_some());
    let second = fx.started("g", "again");
    assert_eq!(second.text, "Messages:\nagain");
    fx.run(second).await;
    assert!(fx.record("stdin.0").starts_with("You are Ada"));
    assert_eq!(fx.record("stdin.1"), "Messages:\nagain");
}

#[tokio::test]
async fn a_member_change_or_a_folder_instructions_edit_sends_the_context_again() {
    let fx = Fixture::new();
    fx.install("");
    let first = fx.started("g", "hello");
    fx.run(first).await;
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
             VALUES ('c', 'Cy', 'red', 'claude-code', 'haiku', '/c', 1, 1);
             INSERT INTO group_members VALUES ('g', 'c', 2, 1);",
        )
        .unwrap()
    });
    let second = fx.started("g", "second");
    assert!(second.text.starts_with(
        "The group details changed:\nYou are Ada, a member of the group chat \"Team\". Members: Ada (you, lead), Bo, Cy."
    ));
    fx.run(second).await;
    let third = fx.started("g", "third");
    assert_eq!(third.text, "Messages:\nthird");
    fx.run(third).await;
    fs::write(fx.folder.path().join("CLAUDE.md"), "New rule\n").unwrap();
    let fourth = fx.started("g", "fourth");
    assert!(fourth
        .text
        .starts_with("The group details changed:\nYou are Ada"));
    assert!(fourth
        .text
        .contains("Instructions from the group folder's CLAUDE.md:\nNew rule"));
    fx.run(fourth).await;
    assert_eq!(fx.started("g", "fifth").text, "Messages:\nfifth");
}

const COMPACT_BOUNDARY: &str = r#"echo '{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"auto","pre_tokens":9000}}'"#;

#[tokio::test]
async fn a_compaction_clears_the_context_hash_so_the_next_turn_sends_the_details_again() {
    let fx = Fixture::new();
    fx.install(COMPACT_BOUNDARY);
    let first = fx.started("g", "hello");
    fx.run(first).await;
    assert_eq!(hash_of(&fx), None);
    let second = fx.started("g", "again");
    assert!(second.text.starts_with("You are Ada"));
    assert!(!second.text.contains("The group details changed:"));
    assert!(second.text.ends_with("Messages:\nagain"));
    fx.run(second).await;
    assert!(fx.record("stdin.1").starts_with("You are Ada"));
}

#[tokio::test]
async fn a_turn_without_a_compaction_keeps_the_context_hash() {
    let fx = Fixture::new();
    fx.install("");
    let first = fx.started("g", "hello");
    fx.run(first).await;
    let hash = hash_of(&fx);
    assert!(hash.is_some());
    let second = fx.started("g", "again");
    fx.run(second).await;
    assert_eq!(hash_of(&fx), hash);
}

#[tokio::test]
async fn a_compaction_in_a_group_turn_leaves_other_groups_of_the_agent_alone() {
    let fx = Fixture::new();
    fx.install("");
    let other = fx.started("g2", "crew");
    fx.run(other).await;
    fx.install(COMPACT_BOUNDARY);
    let first = fx.started("g", "hello");
    fx.run(first).await;
    assert_eq!(hash_of(&fx), None);
    let crew: Option<String> = fx.value(
        "SELECT context_hash FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g2'",
        &[],
    );
    assert!(crew.is_some());
}

#[tokio::test]
async fn a_reset_session_gets_the_context_again() {
    let fx = Fixture::new();
    fx.install("case \"$*\" in *--resume*) echo 'No conversation found' >&2; exit 1;; esac");
    let first = fx.started("g", "hello");
    fx.run(first).await;
    let second = fx.started("g", "again");
    assert_eq!(second.text, "Messages:\nagain");
    fx.run(second).await;
    assert_eq!(fx.record("stdin.1"), "Messages:\nagain");
    assert!(fx.record("stdin.2").starts_with("You are Ada"));
    assert!(!fx.record("stdin.2").contains("The group details changed:"));
    assert!(fx.record("stdin.2").ends_with("Messages:\nagain"));
}

#[tokio::test]
async fn a_turn_that_never_reached_the_session_start_stores_no_context_hash() {
    let fx = Fixture::new();
    fx.install("exit 3");
    let first = fx.started("g", "hello");
    fx.run(first).await;
    assert_eq!(hash_of(&fx), None);
    let second = fx.started("g", "again");
    assert!(second.text.starts_with("You are Ada"));
}

#[tokio::test]
async fn a_refused_folder_instructions_file_is_reported_as_a_notice_of_the_turn() {
    let fx = Fixture::new();
    fx.install("");
    fs::write(
        fx.folder.path().join("CLAUDE.md"),
        "x".repeat(super::FOLDER_INSTRUCTIONS_MAX_BYTES as usize + 1),
    )
    .unwrap();
    let launch = fx.started("g", "hello");
    let turn_id = launch.turn_id.clone();
    fx.run(launch).await;
    assert_eq!(
        fx.value::<String>(
            "SELECT content FROM messages WHERE turn_id = ?1 AND role = 'notice'",
            &[&turn_id]
        ),
        "The group folder's CLAUDE.md was not loaded (it is larger than 64 KiB)."
    );
    assert_eq!(
        fx.value::<String>("SELECT status FROM turns WHERE id = ?1", &[&turn_id]),
        "completed"
    );
}

#[tokio::test]
async fn a_members_unusable_workspace_instruction_file_is_reported_as_a_notice_of_the_group_turn() {
    let fx = Fixture::new();
    fx.install("");
    fs::write(
        fx.workspaces.path().join("Ada").join("CLAUDE.md"),
        [0xff, 0xfe],
    )
    .unwrap();
    let launch = fx.started("g", "hello");
    let turn_id = launch.turn_id.clone();
    fx.run(launch).await;
    assert_eq!(
        fx.value::<String>(
            "SELECT content FROM messages WHERE turn_id = ?1 AND role = 'notice'",
            &[&turn_id]
        ),
        "The workspace's CLAUDE.md was not loaded (it is not UTF-8 text)."
    );
}

#[tokio::test]
async fn the_shutdown_before_the_session_start_restores_the_watermark_and_spawns_nothing() {
    let fx = Fixture::new();
    fx.install("");
    let seen = || {
        fx.value::<i64>(
            "SELECT last_seen_seq FROM agent_group_sessions WHERE agent_id = 'a' AND group_id = 'g'",
            &[],
        )
    };
    let launch = fx.started("g", "hello");
    assert!(seen() > 0);
    fx.registry
        .closing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fx.run(launch).await;
    assert_eq!(seen(), 0);
    assert_eq!(
        fx.value::<String>("SELECT status FROM turns", &[]),
        "running"
    );
    assert!(!fx.records.path().join("args.0").exists());
}

#[tokio::test]
async fn a_shutdown_during_the_help_probe_stops_the_spawn() {
    let fx = Fixture::new();
    fx.install("");
    fs::write(fx.records.path().join("help-delay"), "").unwrap();
    let launch = fx.started("g", "hello");
    let closing = async {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        fx.registry
            .closing
            .store(true, std::sync::atomic::Ordering::SeqCst);
    };
    tokio::join!(fx.run(launch), closing);
    assert!(!fx.records.path().join("args.0").exists());
    assert_eq!(
        fx.value::<String>("SELECT status FROM turns", &[]),
        "running"
    );
}

#[test]
fn an_addressed_agent_that_left_the_group_is_refused_or_re_addressed_to_the_lead() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
             VALUES ('c', 'Cy', 'red', 'claude-code', 'haiku', '/c', 1, 1);",
        )
        .unwrap()
    });
    let refused = fx
        .db
        .with_connection(|c| begin_in(c, "g", "c", "hi", &Origin::user("r", "c", false), 5))
        .err()
        .unwrap();
    assert!(matches!(
        refused,
        BeginError::Refused {
            code: "not-member",
            ..
        }
    ));
    assert_eq!(fx.value::<i64>("SELECT COUNT(*) FROM turns", &[]), 0);
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'c', 'for the one who left', 0, 1)",
            [],
        )
        .unwrap()
    });
    let launch = start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").unwrap();
    assert_eq!(launch.agent_id, "a");
    assert_eq!(input(&launch), "for the one who left");
    assert!(fx.queue_texts("g").is_empty());
}

#[test]
fn a_queue_entry_for_an_agent_that_left_a_leaderless_group_is_dropped() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
             VALUES ('c', 'Cy', 'red', 'claude-code', 'haiku', '/c', 1, 1);
             INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'c', 'orphan', 0, 1);
             UPDATE groups SET lead_agent_id = NULL WHERE id = 'g';",
        )
        .unwrap()
    });
    assert!(start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").is_none());
    assert!(fx.queue_texts("g").is_empty());
}

#[test]
fn the_lead_is_resolved_inside_the_begin_transaction() {
    let fx = Fixture::new();
    fx.db
        .with_connection(|c| c.execute("UPDATE groups SET lead_agent_id = 'b' WHERE id = 'g'", []))
        .unwrap();
    let sent = fx
        .db
        .with_connection(|c| begin_or_queue(c, "g", "hello", 5))
        .ok()
        .unwrap();
    let GroupSent::Started(begun) = sent else {
        panic!("expected a started turn");
    };
    assert_eq!(begun.agent_id, "b");
}

#[test]
fn a_failed_one_to_one_chat_of_another_agent_does_not_skip_this_agents_own_queue() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q1', 'g', 'b', 'for bo', 0, 1), ('q2', 'g', 'a', 'for ada', 1, 1);",
        )
        .unwrap()
    });
    let mut chain = Chain::default();
    chain.record("b", None, RunEnd::ChatFailed);
    let bo_turn = start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").unwrap();
    assert_eq!(bo_turn.agent_id, "b");
    fx.end(&bo_turn, "completed");
    let hop = fx.after_in("b", Some("g"), RunEnd::Finished, &chain);
    assert_eq!(hop.len(), 1);
    assert_eq!(hop[0].agent_id, "a");
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO queued_messages (id, agent_id, content, position, created_at)
             VALUES ('p', 'a', 'private', 0, 1)",
            [],
        )
        .unwrap()
    });
    fx.end(&hop[0], "completed");
    let after_group = fx.after_in("a", Some("g"), RunEnd::Finished, &chain);
    assert_eq!(after_group.len(), 1);
    assert_eq!(after_group[0].agent_id, "a");
    assert!(after_group[0].group.is_none());
    assert_eq!(after_group[0].text, "private");
}

#[test]
fn resume_releases_the_hold_so_an_old_chain_can_start_the_agents_entry() {
    let fx = Fixture::new();
    fx.db
        .with_connection(|c| c.execute("UPDATE groups SET lead_agent_id = 'b' WHERE id = 'g2'", []))
        .unwrap();
    let bo_turn = fx.started("g2", "bo works");
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g2', 'a', 'for ada', 0, 1)",
            [],
        )
        .unwrap()
    });
    fx.registry.hold("a");
    fx.end(&bo_turn, "completed");
    assert!(fx.after("b", Some("g2"), RunEnd::Finished).is_empty());
    assert!(resume(&fx.db, &fx.registry, &fx.dirs(), "a")
        .unwrap()
        .is_none());
    assert!(!fx.registry.is_held("a"));
    let launches = fx.after("b", Some("g2"), RunEnd::Finished);
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].agent_id, "a");
    assert_eq!(input(&launches[0]), "User: bo works\n\nfor ada");
}

#[tokio::test]
async fn a_sign_in_failure_after_the_process_started_holds_the_agent_until_resume() {
    let fx = Fixture::new();
    fx.install("echo 'Not logged in' >&2\nexit 1");
    let launch = fx.started("g", "hello");
    fill_queues(&fx);
    let mut launch = launch;
    launch
        .search_dirs
        .extend(["/usr/bin".into(), "/bin".into()]);
    let end = runner::run(&fx.db, &fx.sink, &fx.registry, launch, ApiKey::Subscription).await;
    assert_eq!(end, RunEnd::AgentFailed);
    note_end(&fx.registry, "a", end);
    for group in [None, Some("g"), Some("g2")] {
        assert!(fx.failed("a", group, end).is_empty(), "{group:?}");
    }
    assert_eq!(fx.queue_texts("g"), ["group one", "group two"]);
    assert_eq!(
        fx.value::<i64>("SELECT COUNT(*) FROM queued_messages", &[]),
        2
    );
    assert!(fx.registry.is_held("a"));
    assert!(resume(&fx.db, &fx.registry, &fx.dirs(), "a")
        .unwrap()
        .is_some());
    assert!(!fx.registry.is_held("a"));
}

#[test]
fn a_held_agent_is_not_started_by_any_drive() {
    let fx = Fixture::new();
    fill_queues(&fx);
    fx.registry.hold("a");
    for end in [RunEnd::Finished, RunEnd::ChatFailed, RunEnd::AgentFailed] {
        for (agent, group) in [
            ("a", None),
            ("a", Some("g")),
            ("b", None),
            ("b", Some("g")),
            ("b", Some("g2")),
        ] {
            let launches = fx.after(agent, group, end);
            assert!(
                launches.iter().all(|launch| launch.agent_id != "a"),
                "{agent} {group:?} {end:?}"
            );
        }
    }
    assert_eq!(started_count(&fx), 0);
}

#[test]
fn the_hold_follows_the_end_of_a_turn_and_the_users_own_actions() {
    let fx = Fixture::new();
    fx.registry.hold("a");
    note_end(&fx.registry, "a", RunEnd::ChatFailed);
    assert!(fx.registry.is_held("a"));
    note_end(&fx.registry, "a", RunEnd::Finished);
    assert!(!fx.registry.is_held("a"));
    note_end(&fx.registry, "a", RunEnd::AgentFailed);
    assert!(fx.registry.is_held("a"));
    fx.send("g", "from the user").unwrap();
    assert!(!fx.registry.is_held("a"));
    fx.registry.hold("a");
    fx.queued("g", "queued by the user");
    assert!(!fx.registry.is_held("a"));
    let fx = Fixture::new();
    fx.registry.hold("a");
    fx.chat("a", "private");
    assert!(!fx.registry.is_held("a"));
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'a', 'waiting', 0, 1)",
            [],
        )
        .unwrap()
    });
    fx.registry.hold("a");
    resume_group(&fx.db, &fx.registry, &fx.dirs(), "g").unwrap();
    assert!(!fx.registry.is_held("a"));
}

#[test]
fn the_hold_is_checked_against_the_agent_the_entry_is_re_addressed_to() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
             VALUES ('c', 'Cy', 'red', 'claude-code', 'haiku', '/c', 1, 1);
             INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
             VALUES ('q', 'g', 'c', 'for the one who left', 0, 1);",
        )
        .unwrap()
    });
    fx.registry.hold("a");
    assert!(fx.after("b", Some("g"), RunEnd::Finished).is_empty());
    assert_eq!(fx.queue_texts("g"), ["for the one who left"]);
    fx.registry.release("a");
    assert_eq!(fx.after("b", Some("g"), RunEnd::Finished).len(), 1);
}

mod flow;
