use std::sync::Arc;
use std::time::Duration;

use jiff::tz::TimeZone;
use jiff::Timestamp;
use rusqlite::Connection;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use super::runs::{self, Mark, RunKind};
use super::schedule::{self, Schedule};
use super::store::{self, Stored};
use super::{clock, zone, RoutineError};
use crate::db::Database;
use crate::turns::{self, TurnRegistry};

pub const ON_TIME_MS: i64 = 2 * 60 * 1000;
pub const LATE_LIMIT_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_WAIT: Duration = Duration::from_secs(60);
const MIN_WAIT: Duration = Duration::from_millis(250);
const E2E_WAIT: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct Scheduler {
    notify: Notify,
}

pub fn wake(app: &AppHandle) {
    if let Some(scheduler) = app.try_state::<Arc<Scheduler>>() {
        scheduler.notify.notify_one();
    }
}

#[derive(Debug, Clone)]
pub struct Due {
    pub agent_id: String,
    pub task: String,
    pub mark: Mark,
}

fn skip_reason(scheduled: i64, started_at: i64) -> &'static str {
    if scheduled < started_at {
        runs::REASON_CLOSED
    } else {
        runs::REASON_ASLEEP
    }
}

fn due_of(routine: &Stored, kind: RunKind, scheduled: i64, now: i64) -> Due {
    Due {
        agent_id: routine.agent_id.clone(),
        task: routine.task.clone(),
        mark: Mark::new(&routine.id, &routine.name, kind, Some(scheduled), now),
    }
}

fn settle(
    connection: &Connection,
    routine: &Stored,
    zone: &TimeZone,
    now: Timestamp,
    started_at: i64,
) -> Result<Option<Due>, RoutineError> {
    let Some(scheduled) = routine.next_run_at else {
        return Ok(None);
    };
    let now_ms = now.as_millisecond();
    let lateness = now_ms - scheduled;
    let record = |outcome: &str, reason: Option<&str>| {
        let mark = Mark::new(
            &routine.id,
            &routine.name,
            RunKind::Scheduled,
            Some(scheduled),
            now_ms,
        );
        runs::record_ended(connection, &mark, outcome, reason).map_err(RoutineError::database)
    };
    if matches!(routine.schedule, Schedule::Once { .. }) {
        store::advance(connection, &routine.id, None, true)?;
        return if lateness < ON_TIME_MS {
            Ok(Some(due_of(routine, RunKind::Scheduled, scheduled, now_ms)))
        } else if lateness < LATE_LIMIT_MS {
            Ok(Some(due_of(routine, RunKind::Late, scheduled, now_ms)))
        } else {
            record("missed", None)?;
            Ok(None)
        };
    }
    let next = schedule::next_after(&routine.schedule, now, zone).map(Timestamp::as_millisecond);
    store::advance(connection, &routine.id, next, false)?;
    if lateness < ON_TIME_MS {
        Ok(Some(due_of(routine, RunKind::Scheduled, scheduled, now_ms)))
    } else {
        record("skipped", Some(skip_reason(scheduled, started_at)))?;
        Ok(None)
    }
}

pub fn plan(
    db: &Database,
    registry: &TurnRegistry,
    zone: &TimeZone,
    now: Timestamp,
    started_at: i64,
) -> Result<Vec<Due>, RoutineError> {
    if registry.is_closing() {
        return Ok(Vec::new());
    }
    db.with_connection(|connection| {
        let transaction = connection
            .unchecked_transaction()
            .map_err(RoutineError::database)?;
        let mut dues = Vec::new();
        for routine in store::list_due(&transaction, now.as_millisecond())? {
            dues.extend(settle(&transaction, &routine, zone, now, started_at)?);
        }
        transaction.commit().map_err(RoutineError::database)?;
        Ok(dues)
    })
}

fn wait_for(db: &Database, now: Timestamp) -> Duration {
    if cfg!(feature = "e2e") {
        return E2E_WAIT;
    }
    let earliest = db.with_connection(store::earliest_next_run).ok().flatten();
    let until = earliest.map_or(MAX_WAIT, |at| {
        Duration::from_millis(u64::try_from(at - now.as_millisecond()).unwrap_or(0))
    });
    until.clamp(MIN_WAIT, MAX_WAIT)
}

pub async fn run(app: AppHandle) {
    let clock = clock::current();
    let started_at = clock.now().as_millisecond();
    let Some(scheduler) = app
        .try_state::<Arc<Scheduler>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    loop {
        if app
            .try_state::<TurnRegistry>()
            .is_none_or(|registry| registry.is_closing())
        {
            return;
        }
        let now = clock.now();
        let plan_app = app.clone();
        let dues = tauri::async_runtime::spawn_blocking(move || {
            let (Some(db), Some(registry)) = (
                plan_app.try_state::<Database>(),
                plan_app.try_state::<TurnRegistry>(),
            ) else {
                return None;
            };
            Some(plan(&db, &registry, &zone(), now, started_at).unwrap_or_default())
        })
        .await
        .ok()
        .flatten();
        let Some(dues) = dues else {
            return;
        };
        for due in dues {
            turns::start_routine(app.clone(), due.agent_id, due.task, due.mark).await;
        }
        let Some(wait) = app
            .try_state::<Database>()
            .map(|db| wait_for(&db, clock.now()))
        else {
            return;
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = scheduler.notify.notified() => {}
        }
    }
}
