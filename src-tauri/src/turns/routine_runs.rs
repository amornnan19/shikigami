use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use super::*;
use crate::routines::runs::{self, Mark, RunKind};

struct Fixture {
    _data: tempfile::TempDir,
    bin: tempfile::TempDir,
    db: Database,
    registry: TurnRegistry,
}

impl Fixture {
    fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let claude = bin.path().join("claude");
        fs::write(&claude, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        let db = Database::open(data.path()).unwrap();
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'haiku', '/w', 1, 1);
                 INSERT INTO routines (id, agent_id, name, task, kind, at_time, created_at, updated_at)
                 VALUES ('r', 'a', 'Morning', 'Check the inbox', 'daily', '08:00', 1, 1);",
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

    fn dirs(&self) -> [PathBuf; 1] {
        [self.bin.path().to_path_buf()]
    }

    fn mark(&self, kind: RunKind) -> Mark {
        Mark::new("r", "Morning", kind, Some(5), 7_000)
    }

    fn run(&self, text: &str) -> Result<Option<Prepared>, TurnError> {
        self.run_marked(text, &self.mark(RunKind::Scheduled))
    }

    fn run_marked(&self, text: &str, mark: &Mark) -> Result<Option<Prepared>, TurnError> {
        prepare_routine(&self.db, &self.registry, &self.dirs(), "a", text, mark)
    }

    fn started(&self, text: &str) -> Launch {
        match self.run(text).unwrap() {
            Some(Prepared::Started(launch)) => launch,
            _ => panic!("expected a started turn"),
        }
    }

    fn queued(&self, text: &str) -> QueuedMessage {
        match self.run(text).unwrap() {
            Some(Prepared::Queued { queued, .. }) => queued,
            _ => panic!("expected a queued message"),
        }
    }

    fn user_send(&self, text: &str) -> Prepared {
        prepare(&self.db, &self.registry, &self.dirs(), "a", text).unwrap()
    }

    fn runs(&self) -> Vec<(String, String, Option<String>, Option<String>)> {
        self.db.with_connection(|c| {
            c.prepare("SELECT kind, outcome, reason, turn_id FROM routine_runs ORDER BY rowid")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        })
    }

    fn outcomes(&self) -> Vec<String> {
        self.runs().into_iter().map(|run| run.1).collect()
    }

    fn turn_mark(&self, turn_id: &str) -> (Option<String>, Option<String>) {
        self.db.with_connection(|c| {
            c.query_row(
                "SELECT routine_id, routine_name FROM turns WHERE id = ?1",
                [turn_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        })
    }

    fn count(&self, table: &str) -> i64 {
        self.db.with_connection(|c| {
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
    }

    fn finish(&self, turn_id: &str, status: &str) {
        self.db
            .with_connection(|c| store::finish_turn(c, turn_id, "a", status, None, 9).unwrap());
    }
}

#[test]
fn a_run_on_an_idle_agent_starts_a_marked_turn_and_records_it_running() {
    let fx = Fixture::new();
    let launch = fx.started("Check the inbox");
    assert_eq!(launch.text, "Check the inbox");
    assert_eq!(
        fx.turn_mark(&launch.turn_id),
        (Some("r".to_string()), Some("Morning".to_string()))
    );
    let runs = fx.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, "scheduled");
    assert_eq!(runs[0].1, "running");
    assert_eq!(runs[0].3.as_deref(), Some(launch.turn_id.as_str()));
    let page = fx
        .db
        .with_connection(|c| store::conversation(c, "a", None))
        .unwrap();
    assert_eq!(page.turns[0].routine_name.as_deref(), Some("Morning"));
    assert_eq!(page.turns[0].user_message, "Check the inbox");
}

#[test]
fn a_run_for_a_busy_agent_waits_in_the_queue_then_starts_with_its_marker() {
    let fx = Fixture::new();
    let UserStart(first) = UserStart::of(fx.user_send("hello"));
    fx.queued("Check the inbox");
    assert_eq!(fx.outcomes(), ["queued"]);
    assert_eq!(fx.count("turns"), 1);
    fx.finish(&first, "completed");
    let launch = start_next(&fx.db, &fx.registry, &fx.dirs(), "a").unwrap();
    assert_eq!(launch.text, "Check the inbox");
    assert_eq!(
        fx.turn_mark(&launch.turn_id),
        (Some("r".to_string()), Some("Morning".to_string()))
    );
    let runs = fx.runs();
    assert_eq!(runs[0].1, "running");
    assert_eq!(runs[0].3.as_deref(), Some(launch.turn_id.as_str()));
    assert!(fx
        .db
        .with_connection(|c| store::list_queue(c, "a"))
        .unwrap()
        .is_empty());
}

struct UserStart(String);

impl UserStart {
    fn of(prepared: Prepared) -> Self {
        match prepared {
            Prepared::Started(launch) => Self(launch.turn_id),
            Prepared::Queued { .. } => panic!("expected a started turn"),
        }
    }
}

#[test]
fn a_user_message_is_never_marked() {
    let fx = Fixture::new();
    let UserStart(turn) = UserStart::of(fx.user_send("hello"));
    assert_eq!(fx.turn_mark(&turn), (None, None));
    assert!(fx.runs().is_empty());
}

#[test]
fn a_second_run_while_the_last_is_running_or_queued_is_skipped_and_starts_nothing() {
    let fx = Fixture::new();
    fx.started("one");
    assert!(fx.run("two").unwrap().is_none());
    assert_eq!(fx.outcomes(), ["running", "skipped"]);
    assert_eq!(
        fx.runs()[1].2.as_deref(),
        Some("the last run had not finished")
    );
    assert_eq!(fx.count("turns"), 1);
    assert_eq!(fx.count("queued_messages"), 0);
    let manual = fx.mark(RunKind::Manual);
    assert!(fx.run_marked("three", &manual).unwrap().is_none());
    assert_eq!(fx.runs()[2].0, "manual");
}

#[test]
fn a_queued_run_also_blocks_the_next_one_until_it_is_removed() {
    let fx = Fixture::new();
    fx.user_send("hello");
    let queued = fx.queued("one");
    assert!(fx.run("two").unwrap().is_none());
    assert_eq!(fx.count("queued_messages"), 1);
    let removed = fx
        .db
        .with_connection(|c| store::remove_queued(c, &queued.id, 9))
        .unwrap();
    assert_eq!(removed.as_deref(), Some("a"));
    let runs = fx.runs();
    assert_eq!(runs[0].1, "skipped");
    assert_eq!(runs[0].2.as_deref(), Some("removed from the queue"));
    assert!(matches!(
        fx.run("three").unwrap(),
        Some(Prepared::Queued { .. })
    ));
}

#[test]
fn the_run_outcome_follows_the_turn_status() {
    for (status, outcome) in [
        ("completed", "completed"),
        ("failed", "failed"),
        ("stopped", "stopped"),
    ] {
        let fx = Fixture::new();
        let launch = fx.started("go");
        fx.finish(&launch.turn_id, status);
        assert_eq!(fx.outcomes(), [outcome], "{status}");
        let ended: Option<i64> = fx.db.with_connection(|c| {
            c.query_row("SELECT ended_at FROM routine_runs", [], |row| row.get(0))
                .unwrap()
        });
        assert_eq!(ended, Some(9));
    }
}

#[test]
fn a_run_whose_turn_was_running_at_startup_is_marked_interrupted() {
    let fx = Fixture::new();
    fx.started("go");
    assert_eq!(sweep_interrupted(&fx.db).unwrap(), 1);
    assert_eq!(fx.outcomes(), ["interrupted"]);
}

#[test]
fn a_held_agent_queues_the_run_and_it_stays_queued() {
    let fx = Fixture::new();
    fx.registry.hold("a");
    fx.queued("go");
    assert_eq!(fx.outcomes(), ["queued"]);
    assert_eq!(fx.count("turns"), 0);
    assert!(fx.registry.is_held("a"));
}

#[test]
fn a_run_never_starts_the_head_of_a_queue_that_is_waiting() {
    let fx = Fixture::new();
    let UserStart(first) = UserStart::of(fx.user_send("one"));
    fx.user_send("two");
    fx.finish(&first, "failed");
    fx.queued("go");
    assert_eq!(fx.count("turns"), 1);
    assert_eq!(fx.count("queued_messages"), 2);
    assert_eq!(fx.outcomes(), ["queued"]);
}

#[test]
fn a_paused_routine_can_still_be_run_by_hand() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute("UPDATE routines SET active = 0", []).unwrap();
    });
    let manual = fx.mark(RunKind::Manual);
    let Ok(Some(Prepared::Started(_))) = fx.run_marked("go", &manual) else {
        panic!("expected a started turn");
    };
    let runs = fx.runs();
    assert_eq!(
        (runs[0].0.as_str(), runs[0].1.as_str()),
        ("manual", "running")
    );
}

#[test]
fn a_closing_registry_starts_and_records_nothing() {
    let fx = Fixture::new();
    fx.registry.kill_all(Duration::ZERO);
    let error = fx.run("go").err().expect("expected an error");
    assert_eq!(error.code, "shutting-down");
    assert_eq!(fx.count("turns"), 0);
    assert_eq!(fx.count("queued_messages"), 0);
    assert!(fx.runs().is_empty());
}

#[test]
fn a_closing_registry_records_nothing_even_when_the_provider_is_unavailable() {
    let fx = Fixture::new();
    fs::remove_file(fx.bin.path().join("claude")).unwrap();
    fx.registry.kill_all(Duration::ZERO);
    let error = fx.run("go").err().expect("expected an error");
    assert_eq!(error.code, "shutting-down");
    assert!(fx.runs().is_empty());
}

#[test]
fn an_unavailable_provider_records_the_run_as_failed() {
    let fx = Fixture::new();
    fs::remove_file(fx.bin.path().join("claude")).unwrap();
    let error = fx.run("go").err().expect("expected an error");
    assert_eq!(error.code, "provider-unavailable");
    let runs = fx.runs();
    assert_eq!(runs[0].1, "failed");
    assert_eq!(
        runs[0].2.as_deref(),
        Some("This agent's provider is not available.")
    );
}

#[test]
fn deleting_a_routine_keeps_the_marker_on_past_turns_and_drops_its_runs_and_queued_runs() {
    let fx = Fixture::new();
    let UserStart(first) = UserStart::of(fx.user_send("hello"));
    fx.queued("waiting");
    fx.finish(&first, "completed");
    let launch = start_next(&fx.db, &fx.registry, &fx.dirs(), "a").unwrap();
    fx.finish(&launch.turn_id, "completed");
    fx.user_send("again");
    fx.run("waiting again").unwrap();
    assert_eq!(fx.count("queued_messages"), 1);
    fx.db.with_connection(|c| {
        c.execute("DELETE FROM routines WHERE id = 'r'", [])
            .unwrap();
    });
    assert_eq!(fx.count("queued_messages"), 0);
    assert_eq!(fx.count("routine_runs"), 0);
    assert_eq!(
        fx.turn_mark(&launch.turn_id),
        (None, Some("Morning".to_string()))
    );
    let page = fx
        .db
        .with_connection(|c| store::conversation(c, "a", None))
        .unwrap();
    assert!(page
        .turns
        .iter()
        .any(|turn| turn.routine_name.as_deref() == Some("Morning")));
}

#[test]
fn the_history_keeps_the_last_twenty_runs_and_never_drops_one_still_going() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        c.execute_batch(
            "INSERT INTO turns (id, agent_id, status, started_at) VALUES ('t1', 'a', 'running', 1);",
        )
        .unwrap();
        let mark = Mark::new("r", "Morning", RunKind::Scheduled, None, 2);
        runs::record_running(c, &mark, "t1").unwrap();
        for at in 10..40 {
            let skip = Mark::new("r", "Morning", RunKind::Scheduled, None, at);
            runs::record_ended(c, &skip, "skipped", Some("x")).unwrap();
        }
    });
    let total = fx.count("routine_runs");
    assert_eq!(total, 21);
    let listed = fx
        .db
        .with_connection(|c| {
            runs::list(
                c,
                "r",
                &jiff::tz::TimeZone::UTC,
                jiff::Timestamp::from_millisecond(0).unwrap(),
            )
        })
        .unwrap();
    assert_eq!(listed.len(), 20);
    assert_eq!(listed[0].created_at, 39);
    assert!(runs_is_active(&fx));
}

fn runs_is_active(fx: &Fixture) -> bool {
    fx.db.with_connection(|c| runs::is_active(c, "r")).unwrap()
}

#[test]
fn only_twenty_finished_runs_stay_after_many_runs() {
    let fx = Fixture::new();
    fx.db.with_connection(|c| {
        for at in 1..=45 {
            let skip = Mark::new("r", "Morning", RunKind::Scheduled, None, at);
            runs::record_ended(c, &skip, "skipped", Some("x")).unwrap();
        }
    });
    assert_eq!(fx.count("routine_runs"), 20);
    let oldest: i64 = fx.db.with_connection(|c| {
        c.query_row("SELECT MIN(created_at) FROM routine_runs", [], |row| {
            row.get(0)
        })
        .unwrap()
    });
    assert_eq!(oldest, 26);
}
