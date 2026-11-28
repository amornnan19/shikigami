use jiff::tz::TimeZone;
use jiff::Timestamp;

use rusqlite::params;

use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    utc: TimeZone,
    now: Timestamp,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    add_agent(&db, "a");
    add_agent(&db, "b");
    Fixture {
        _dir: dir,
        db,
        utc: TimeZone::get("UTC").unwrap(),
        now: "2026-11-28T10:00:00Z".parse().unwrap(),
    }
}

fn add_agent(db: &Database, id: &str) {
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
             VALUES (?1, ?1, 'red', 'claude-code', 'm', ?1, 1, 1)",
            [id],
        )
        .unwrap()
    });
}

fn daily(name: &str) -> RoutineInput {
    RoutineInput {
        name: name.to_string(),
        task: "Check the inbox".to_string(),
        schedule: ScheduleDto::Daily {
            time: "08:00".to_string(),
            days: 127,
        },
    }
}

fn once(name: &str, date: &str, time: &str) -> RoutineInput {
    RoutineInput {
        schedule: ScheduleDto::Once {
            date: date.to_string(),
            time: time.to_string(),
        },
        ..daily(name)
    }
}

fn code<T>(result: Result<T, RoutineError>) -> &'static str {
    result.err().expect("expected an error").code
}

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}

impl Fixture {
    fn create(&self, agent: &str, input: RoutineInput) -> Result<Routine, RoutineError> {
        create(&self.db, &self.utc, self.now, agent, input)
    }

    fn stored_next(&self, id: &str) -> Option<i64> {
        self.db.with_connection(|c| {
            c.query_row(
                "SELECT next_run_at FROM routines WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
        })
    }
}

#[test]
fn a_new_routine_is_active_and_knows_its_description_and_next_run() {
    let f = fixture();
    let routine = f.create("a", daily("  Morning  ")).unwrap();
    assert_eq!(routine.name, "Morning");
    assert_eq!(routine.description, "Every day at 08:00");
    assert!(routine.active && !routine.done);
    assert_eq!(routine.next_run_at, Some(ms("2026-11-29T08:00:00Z")));
    assert_eq!(
        routine.next_run_text.as_deref(),
        Some("Sun 29 Nov 2026, 08:00")
    );
    assert_eq!(routine.last_run, None);
    assert_eq!(f.stored_next(&routine.id), routine.next_run_at);
    let listed = list(&f.db, &f.utc, f.now, "a").unwrap();
    assert_eq!(listed, vec![routine]);
    assert!(list(&f.db, &f.utc, f.now, "b").unwrap().is_empty());
}

#[test]
fn every_input_field_is_validated_with_its_own_code() {
    let f = fixture();
    let mut cases: Vec<(&str, RoutineInput)> = vec![
        ("name-empty", daily("  ")),
        ("name-too-long", daily(&"n".repeat(61))),
        ("name-invalid", daily("a\nb")),
        (
            "task-empty",
            RoutineInput {
                task: " ".to_string(),
                ..daily("x")
            },
        ),
        (
            "task-too-long",
            RoutineInput {
                task: "t".repeat(20_001),
                ..daily("x")
            },
        ),
        ("time-invalid", once("x", "2026-12-03", "25:00")),
        ("date-invalid", once("x", "2026-02-30", "08:00")),
        ("date-past", once("x", "2026-11-28", "09:59")),
    ];
    let schedule_cases = [
        (
            "days-empty",
            ScheduleDto::Daily {
                time: "08:00".to_string(),
                days: 0,
            },
        ),
        (
            "hours-invalid",
            ScheduleDto::Hourly {
                time: "08:00".to_string(),
                every_hours: 13,
                days: 127,
            },
        ),
        (
            "weekday-invalid",
            ScheduleDto::Weekly {
                weekday: 8,
                time: "08:00".to_string(),
            },
        ),
    ];
    for (expected, schedule) in schedule_cases {
        cases.push((
            expected,
            RoutineInput {
                schedule,
                ..daily("x")
            },
        ));
    }
    for (expected, input) in cases {
        assert_eq!(code(f.create("a", input)), expected);
    }
    assert!(list(&f.db, &f.utc, f.now, "a").unwrap().is_empty());
}

#[test]
fn a_once_routine_at_the_current_minute_or_earlier_is_in_the_past() {
    let f = fixture();
    assert_eq!(
        code(f.create("a", once("x", "2026-11-28", "10:00"))),
        "date-past"
    );
    assert_eq!(
        code(f.create("a", once("x", "2026-11-27", "23:00"))),
        "date-past"
    );
    let routine = f.create("a", once("x", "2026-11-28", "10:01")).unwrap();
    assert_eq!(routine.description, "Once on 28 Nov 2026 at 10:01");
    assert_eq!(routine.next_run_at, Some(ms("2026-11-28T10:01:00Z")));
}

#[test]
fn an_agent_has_at_most_thirty_two_routines() {
    let f = fixture();
    for index in 0..32 {
        f.create("a", daily(&format!("Routine {index}"))).unwrap();
    }
    assert_eq!(code(f.create("a", daily("One more"))), "too-many-routines");
    assert!(f.create("b", daily("One more")).is_ok());
    let first = list(&f.db, &f.utc, f.now, "a").unwrap().remove(0);
    delete(&f.db, &first.id).unwrap();
    assert!(f.create("a", daily("One more")).is_ok());
}

#[test]
fn a_routine_name_is_unique_per_agent_ignoring_case() {
    let f = fixture();
    let first = f.create("a", daily("Morning")).unwrap();
    assert_eq!(code(f.create("a", daily("MORNING"))), "name-taken");
    assert!(f.create("b", daily("Morning")).is_ok());
    let second = f.create("a", daily("Evening")).unwrap();
    assert_eq!(
        code(update(&f.db, &f.utc, f.now, &second.id, daily("morning"))),
        "name-taken"
    );
    assert!(update(&f.db, &f.utc, f.now, &first.id, daily("MORNING")).is_ok());
}

#[test]
fn an_unknown_agent_or_routine_is_reported() {
    let f = fixture();
    assert_eq!(code(f.create("missing", daily("x"))), "agent-not-found");
    assert_eq!(
        code(update(&f.db, &f.utc, f.now, "missing", daily("x"))),
        "routine-not-found"
    );
    assert_eq!(
        code(set_active(&f.db, &f.utc, f.now, "missing", false)),
        "routine-not-found"
    );
    assert_eq!(code(delete(&f.db, "missing")), "routine-not-found");
}

#[test]
fn pausing_clears_the_next_run_and_resuming_restores_it() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    let paused = set_active(&f.db, &f.utc, f.now, &routine.id, false).unwrap();
    assert!(!paused.active);
    assert_eq!(paused.next_run_at, None);
    assert_eq!(paused.next_run_text, None);
    assert_eq!(f.stored_next(&routine.id), None);
    let later: Timestamp = "2026-11-29T09:00:00Z".parse().unwrap();
    let resumed = set_active(&f.db, &f.utc, later, &routine.id, true).unwrap();
    assert!(resumed.active);
    assert_eq!(resumed.next_run_at, Some(ms("2026-11-30T08:00:00Z")));
    assert_eq!(f.stored_next(&routine.id), resumed.next_run_at);
}

#[test]
fn editing_changes_the_schedule_and_recomputes_the_next_run() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    let edited = update(
        &f.db,
        &f.utc,
        f.now,
        &routine.id,
        RoutineInput {
            name: "Evening".to_string(),
            task: "Summarise".to_string(),
            schedule: ScheduleDto::Weekly {
                weekday: 1,
                time: "18:30".to_string(),
            },
        },
    )
    .unwrap();
    assert_eq!(edited.name, "Evening");
    assert_eq!(edited.task, "Summarise");
    assert_eq!(edited.description, "Every Monday at 18:30");
    assert_eq!(edited.next_run_at, Some(ms("2026-11-30T18:30:00Z")));
    assert_eq!(edited.created_at, routine.created_at);
    assert_eq!(list(&f.db, &f.utc, f.now, "a").unwrap(), vec![edited]);
}

#[test]
fn editing_a_paused_routine_keeps_it_paused() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    set_active(&f.db, &f.utc, f.now, &routine.id, false).unwrap();
    let edited = update(&f.db, &f.utc, f.now, &routine.id, daily("Morning")).unwrap();
    assert!(!edited.active);
    assert_eq!(edited.next_run_at, None);
}

#[test]
fn editing_a_done_once_routine_to_a_future_time_makes_it_not_done() {
    let f = fixture();
    let routine = f.create("a", once("Once", "2026-11-29", "08:00")).unwrap();
    f.db.with_connection(|c| {
        c.execute(
            "UPDATE routines SET done = 1, next_run_at = NULL WHERE id = ?1",
            [&routine.id],
        )
        .unwrap()
    });
    let same = update(
        &f.db,
        &f.utc,
        "2026-11-30T00:00:00Z".parse().unwrap(),
        &routine.id,
        once("Once renamed", "2026-11-29", "08:00"),
    )
    .unwrap();
    assert!(same.done);
    assert_eq!(same.next_run_at, None);
    let later = "2026-11-30T00:00:00Z".parse().unwrap();
    assert_eq!(
        code(update(
            &f.db,
            &f.utc,
            later,
            &routine.id,
            once("Once", "2026-11-29", "09:00")
        )),
        "date-past"
    );
    let moved = update(
        &f.db,
        &f.utc,
        later,
        &routine.id,
        once("Once", "2026-12-05", "09:00"),
    )
    .unwrap();
    assert!(!moved.done);
    assert_eq!(moved.next_run_at, Some(ms("2026-12-05T09:00:00Z")));
}

#[test]
fn a_missed_once_routine_keeps_its_time_so_it_can_be_run_late() {
    let f = fixture();
    let routine = f.create("a", once("Once", "2026-11-29", "08:00")).unwrap();
    let later: Timestamp = "2026-11-29T12:00:00Z".parse().unwrap();
    refresh_next_runs(&f.db, &f.utc, later).unwrap();
    assert_eq!(f.stored_next(&routine.id), Some(ms("2026-11-29T08:00:00Z")));
    let renamed = update(
        &f.db,
        &f.utc,
        later,
        &routine.id,
        once("Renamed", "2026-11-29", "08:00"),
    )
    .unwrap();
    assert_eq!(renamed.next_run_at, Some(ms("2026-11-29T08:00:00Z")));
}

#[test]
fn refreshing_recomputes_every_next_run_in_the_given_zone() {
    let f = fixture();
    let active = f.create("a", daily("Active")).unwrap();
    let paused = f.create("a", daily("Paused")).unwrap();
    set_active(&f.db, &f.utc, f.now, &paused.id, false).unwrap();
    let tokyo = TimeZone::get("Asia/Tokyo").unwrap();
    refresh_next_runs(&f.db, &tokyo, f.now).unwrap();
    assert_eq!(f.stored_next(&active.id), Some(ms("2026-11-28T23:00:00Z")));
    assert_eq!(f.stored_next(&paused.id), None);
    let tokyo_view = list(&f.db, &tokyo, f.now, "a").unwrap();
    let shown = tokyo_view.iter().find(|r| r.id == active.id).unwrap();
    assert_eq!(
        shown.next_run_text.as_deref(),
        Some("Sun 29 Nov 2026, 08:00")
    );
}

#[test]
fn refreshing_keeps_a_run_time_that_has_already_passed_so_the_scheduler_can_settle_it() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    let later: Timestamp = "2026-12-02T12:00:00Z".parse().unwrap();
    refresh_next_runs(&f.db, &f.utc, later).unwrap();
    assert_eq!(f.stored_next(&routine.id), Some(ms(DUE)));
}

#[test]
fn deleting_a_routine_removes_only_that_routine() {
    let f = fixture();
    let first = f.create("a", daily("One")).unwrap();
    f.create("a", daily("Two")).unwrap();
    delete(&f.db, &first.id).unwrap();
    let names: Vec<_> = list(&f.db, &f.utc, f.now, "a")
        .unwrap()
        .into_iter()
        .map(|routine| routine.name)
        .collect();
    assert_eq!(names, ["Two"]);
}

#[test]
fn deleting_an_agent_deletes_its_routines_and_no_others() {
    let f = fixture();
    f.create("a", daily("One")).unwrap();
    f.create("b", daily("One")).unwrap();
    crate::agents::remove::delete(&f.db, &Err(String::new()), "a", false, &|_| Ok(())).unwrap();
    assert!(list(&f.db, &f.utc, f.now, "a").unwrap().is_empty());
    assert_eq!(list(&f.db, &f.utc, f.now, "b").unwrap().len(), 1);
}

#[test]
fn the_time_zone_override_is_honoured_in_tests() {
    assert_eq!(
        zone_name(&TimeZone::get("Europe/Berlin").unwrap()),
        "Europe/Berlin"
    );
}

#[test]
fn the_input_and_output_use_camel_case_json() {
    let input: RoutineInput = serde_json::from_str(
        r#"{"name":"n","task":"t","schedule":{"kind":"hourly","time":"09:00","everyHours":3,"days":127}}"#,
    )
    .unwrap();
    assert_eq!(
        input.schedule,
        ScheduleDto::Hourly {
            time: "09:00".to_string(),
            every_hours: 3,
            days: 127
        }
    );
    let f = fixture();
    let routine = f.create("a", input).unwrap();
    let json = serde_json::to_value(&routine).unwrap();
    assert_eq!(json["schedule"]["everyHours"], 3);
    assert_eq!(json["agentId"], "a");
    assert_eq!(json["lastRun"], serde_json::Value::Null);
    assert_eq!(json["description"], "Every 3 hours from 09:00, every day");
}

const DUE: &str = "2026-11-29T08:00:00Z";

impl Fixture {
    fn plan_at(&self, now: &str, started_at: &str) -> Vec<scheduler::Due> {
        scheduler::plan(
            &self.db,
            &TurnRegistry::default(),
            &self.utc,
            now.parse().unwrap(),
            ms(started_at),
        )
        .unwrap()
    }

    fn plan_after(&self, now_ms: i64) -> Vec<scheduler::Due> {
        let now = Timestamp::from_millisecond(now_ms).unwrap();
        scheduler::plan(
            &self.db,
            &TurnRegistry::default(),
            &self.utc,
            now,
            ms("2026-11-28T10:00:00Z"),
        )
        .unwrap()
    }

    fn runs(&self, id: &str) -> Vec<(String, String, Option<String>)> {
        self.db.with_connection(|c| {
            c.prepare(
                "SELECT kind, outcome, reason FROM routine_runs WHERE routine_id = ?1 ORDER BY rowid",
            )
            .unwrap()
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
        })
    }

    fn done(&self, id: &str) -> bool {
        self.db.with_connection(|c| {
            c.query_row("SELECT done FROM routines WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
        })
    }
}

fn run(kind: &str, outcome: &str, reason: Option<&str>) -> (String, String, Option<String>) {
    (
        kind.to_string(),
        outcome.to_string(),
        reason.map(str::to_string),
    )
}

#[test]
fn a_repeating_routine_on_time_starts_a_run_and_moves_to_its_next_time() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    assert!(f
        .plan_at("2026-11-29T07:59:59Z", "2026-11-28T10:00:00Z")
        .is_empty());
    let dues = f.plan_at("2026-11-29T08:00:30Z", "2026-11-28T10:00:00Z");
    assert_eq!(dues.len(), 1);
    assert_eq!(dues[0].agent_id, "a");
    assert_eq!(dues[0].task, "Check the inbox");
    assert_eq!(dues[0].mark.routine_name, "Morning");
    assert_eq!(dues[0].mark.kind, RunKind::Scheduled);
    assert_eq!(dues[0].mark.scheduled_for, Some(ms(DUE)));
    assert_eq!(f.stored_next(&routine.id), Some(ms("2026-11-30T08:00:00Z")));
    assert!(!f.done(&routine.id));
    assert!(f.runs(&routine.id).is_empty());
}

#[test]
fn a_run_two_minutes_late_is_no_longer_on_time_but_one_millisecond_less_is() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    assert_eq!(f.plan_after(ms(DUE) + 2 * 60 * 1000 - 1).len(), 1);
    let other = fixture();
    let routine_two = other.create("a", daily("Morning")).unwrap();
    assert!(other.plan_after(ms(DUE) + 2 * 60 * 1000).is_empty());
    assert_eq!(
        other.runs(&routine_two.id),
        vec![run("scheduled", "skipped", Some("the Mac was asleep"))]
    );
    assert!(f.runs(&routine.id).is_empty());
}

#[test]
fn a_late_repeating_routine_records_one_skip_and_continues_at_its_next_time() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    let dues = f.plan_at("2026-12-01T09:30:00Z", "2026-12-01T09:00:00Z");
    assert!(dues.is_empty());
    assert_eq!(
        f.runs(&routine.id),
        vec![run(
            "scheduled",
            "skipped",
            Some("Shikigami was not running")
        )]
    );
    assert_eq!(f.stored_next(&routine.id), Some(ms("2026-12-02T08:00:00Z")));
    assert!(f
        .plan_at("2026-12-01T09:31:00Z", "2026-12-01T09:00:00Z")
        .is_empty());
    assert_eq!(f.runs(&routine.id).len(), 1);
}

#[test]
fn a_time_missed_while_the_app_was_open_is_recorded_as_the_mac_being_asleep() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    assert!(f
        .plan_at("2026-11-29T11:00:00Z", "2026-11-28T10:00:00Z")
        .is_empty());
    assert_eq!(
        f.runs(&routine.id),
        vec![run("scheduled", "skipped", Some("the Mac was asleep"))]
    );
    assert_eq!(f.stored_next(&routine.id), Some(ms("2026-11-30T08:00:00Z")));
}

#[test]
fn a_once_routine_runs_on_time_then_is_done_with_no_next_run() {
    let f = fixture();
    let routine = f
        .create("a", once("Launch", "2026-11-29", "08:00"))
        .unwrap();
    let dues = f.plan_at("2026-11-29T08:00:10Z", "2026-11-28T10:00:00Z");
    assert_eq!(dues.len(), 1);
    assert_eq!(dues[0].mark.kind, RunKind::Scheduled);
    assert!(f.done(&routine.id));
    assert_eq!(f.stored_next(&routine.id), None);
    assert!(f
        .plan_at("2026-11-29T09:00:00Z", "2026-11-28T10:00:00Z")
        .is_empty());
    let listed = list(&f.db, &f.utc, f.now, "a").unwrap();
    assert!(listed[0].done);
}

#[test]
fn a_once_routine_is_on_time_until_two_minutes_and_late_from_then() {
    let f = fixture();
    f.create("a", once("Launch", "2026-11-29", "08:00"))
        .unwrap();
    let dues = f.plan_after(ms(DUE) + 2 * 60 * 1000 - 1);
    assert_eq!(dues[0].mark.kind, RunKind::Scheduled);
    let other = fixture();
    other
        .create("a", once("Launch", "2026-11-29", "08:00"))
        .unwrap();
    let dues = other.plan_after(ms(DUE) + 2 * 60 * 1000);
    assert_eq!(dues[0].mark.kind, RunKind::Late);
}

#[test]
fn a_once_routine_missed_by_less_than_a_day_runs_late() {
    let f = fixture();
    let routine = f
        .create("a", once("Launch", "2026-11-29", "08:00"))
        .unwrap();
    let dues = f.plan_after(ms(DUE) + 24 * 60 * 60 * 1000 - 1);
    assert_eq!(dues.len(), 1);
    assert_eq!(dues[0].mark.kind, RunKind::Late);
    assert_eq!(dues[0].mark.scheduled_for, Some(ms(DUE)));
    assert!(f.done(&routine.id));
    assert_eq!(f.stored_next(&routine.id), None);
}

#[test]
fn a_once_routine_missed_by_a_day_or_more_is_recorded_missed_and_done() {
    let f = fixture();
    let routine = f
        .create("a", once("Launch", "2026-11-29", "08:00"))
        .unwrap();
    assert!(f.plan_after(ms(DUE) + 24 * 60 * 60 * 1000).is_empty());
    assert_eq!(f.runs(&routine.id), vec![run("scheduled", "missed", None)]);
    assert!(f.done(&routine.id));
    assert_eq!(f.stored_next(&routine.id), None);
}

#[test]
fn paused_and_done_routines_are_never_due() {
    let f = fixture();
    let paused = f.create("a", daily("Paused")).unwrap();
    set_active(&f.db, &f.utc, f.now, &paused.id, false).unwrap();
    let finished = f
        .create("a", once("Finished", "2026-11-29", "08:00"))
        .unwrap();
    f.db.with_connection(|c| {
        c.execute(
            "UPDATE routines SET done = 1, next_run_at = ?2 WHERE id = ?1",
            params![finished.id, ms(DUE)],
        )
        .unwrap()
    });
    assert!(f
        .plan_at("2026-11-29T08:00:10Z", "2026-11-28T10:00:00Z")
        .is_empty());
    assert!(f.runs(&paused.id).is_empty());
    assert!(f.runs(&finished.id).is_empty());
}

#[test]
fn a_closing_registry_plans_nothing_and_changes_nothing() {
    let f = fixture();
    let routine = f.create("a", daily("Morning")).unwrap();
    let registry = TurnRegistry::default();
    registry.kill_all(std::time::Duration::ZERO);
    let dues = scheduler::plan(
        &f.db,
        &registry,
        &f.utc,
        "2026-11-29T08:00:10Z".parse().unwrap(),
        0,
    )
    .unwrap();
    assert!(dues.is_empty());
    assert_eq!(f.stored_next(&routine.id), Some(ms(DUE)));
    assert!(f.runs(&routine.id).is_empty());
}

#[test]
fn routines_due_together_start_in_order_of_time_then_name() {
    let f = fixture();
    f.create("a", daily("Zed")).unwrap();
    f.create("b", daily("Alpha")).unwrap();
    f.create(
        "a",
        RoutineInput {
            schedule: ScheduleDto::Daily {
                time: "07:59".to_string(),
                days: 127,
            },
            ..daily("Late name")
        },
    )
    .unwrap();
    let dues = f.plan_after(ms(DUE) + 1000);
    let names: Vec<_> = dues
        .iter()
        .map(|due| due.mark.routine_name.as_str())
        .collect();
    assert_eq!(names, ["Late name", "Alpha", "Zed"]);
}
