pub(crate) mod clock;
pub(crate) mod runs;
pub(crate) mod schedule;
pub(crate) mod scheduler;
mod store;
pub(crate) mod validate;

use jiff::tz::TimeZone;
use jiff::Timestamp;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::db::Database;
use crate::turns::{self, TurnRegistry};
use runs::{Mark, RunKind, RunView};
use schedule::Schedule;
use store::Stored;

pub const ROUTINES_PER_AGENT: i64 = 32;

#[cfg(any(test, feature = "e2e"))]
pub const TZ_ENV: &str = "SHIKIGAMI_TZ";

const NAME_COLUMNS: &str = "routines.name";

#[derive(Debug, Serialize)]
pub struct RoutineError {
    pub code: &'static str,
    pub message: String,
}

impl RoutineError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn database(error: impl std::fmt::Display) -> Self {
        Self::new(
            "database-failed",
            format!("Could not save the routine: {error}"),
        )
    }

    fn stored(id: &str) -> Self {
        Self::new(
            "routine-unreadable",
            format!("Routine {id} has a schedule that could not be read."),
        )
    }

    fn not_found() -> Self {
        Self::new("routine-not-found", "This routine no longer exists.")
    }

    fn save(error: rusqlite::Error) -> Self {
        match &error {
            rusqlite::Error::SqliteFailure(failure, Some(message))
                if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    && message.contains(NAME_COLUMNS) =>
            {
                Self::new(
                    "name-taken",
                    "This agent already has a routine with this name.",
                )
            }
            rusqlite::Error::SqliteFailure(failure, _)
                if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
            {
                Self::agent_not_found()
            }
            _ => Self::database(error),
        }
    }

    fn agent_not_found() -> Self {
        Self::new("agent-not-found", "This agent no longer exists.")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ScheduleDto {
    Once {
        date: String,
        time: String,
    },
    Daily {
        time: String,
        days: i64,
    },
    Hourly {
        time: String,
        every_hours: i64,
        days: i64,
    },
    Weekly {
        weekday: i64,
        time: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInput {
    pub name: String,
    pub task: String,
    pub schedule: ScheduleDto,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub task: String,
    pub schedule: ScheduleDto,
    pub description: String,
    pub active: bool,
    pub done: bool,
    pub next_run_at: Option<i64>,
    pub next_run_text: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run: Option<RunView>,
}

pub fn zone() -> TimeZone {
    #[cfg(any(test, feature = "e2e"))]
    if let Some(zone) = std::env::var(TZ_ENV)
        .ok()
        .filter(|name| !name.is_empty())
        .and_then(|name| TimeZone::get(&name).ok())
    {
        return zone;
    }
    TimeZone::system()
}

pub fn zone_name(zone: &TimeZone) -> String {
    zone.iana_name()
        .map_or_else(|| "your Mac's time zone".to_string(), str::to_string)
}

fn with_last(
    connection: &Connection,
    mut routine: Routine,
    zone: &TimeZone,
    now: Timestamp,
) -> Result<Routine, RoutineError> {
    routine.last_run =
        runs::last(connection, &routine.id, zone, now).map_err(RoutineError::database)?;
    Ok(routine)
}

fn timestamp_of(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn next_run(
    schedule: &Schedule,
    active: bool,
    done: bool,
    now: Timestamp,
    zone: &TimeZone,
) -> Option<i64> {
    if !active || done {
        return None;
    }
    let next = match schedule {
        Schedule::Once { .. } => schedule::once_instant(schedule, zone),
        _ => schedule::next_after(schedule, now, zone),
    };
    next.map(Timestamp::as_millisecond)
}

fn view(stored: Stored, zone: &TimeZone) -> Routine {
    let next_run_text = stored.next_run_at.map(|ms| {
        timestamp_of(ms)
            .to_zoned_datetime(zone.clone())
            .strftime("%a %-d %b %Y, %H:%M")
            .to_string()
    });
    Routine {
        description: schedule::describe(&stored.schedule),
        schedule: validate::to_dto(&stored.schedule),
        id: stored.id,
        agent_id: stored.agent_id,
        name: stored.name,
        task: stored.task,
        active: stored.active,
        done: stored.done,
        next_run_at: stored.next_run_at,
        next_run_text,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
        last_run: None,
    }
}

fn check_not_past(
    schedule: &Schedule,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<(), RoutineError> {
    match schedule::once_instant(schedule, zone) {
        Some(instant) if instant > now => Ok(()),
        _ if !matches!(schedule, Schedule::Once { .. }) => Ok(()),
        _ => Err(RoutineError::new(
            "date-past",
            "Pick a date and time that is still ahead.",
        )),
    }
}

pub fn list(
    db: &Database,
    zone: &TimeZone,
    now: Timestamp,
    agent_id: &str,
) -> Result<Vec<Routine>, RoutineError> {
    db.with_connection(|connection| {
        store::list_for_agent(connection, agent_id)?
            .into_iter()
            .map(|stored| with_last(connection, view(stored, zone), zone, now))
            .collect()
    })
}

pub fn history(
    db: &Database,
    zone: &TimeZone,
    now: Timestamp,
    routine_id: &str,
) -> Result<Vec<RunView>, RoutineError> {
    db.with_connection(|connection| {
        runs::list(connection, routine_id, zone, now).map_err(RoutineError::database)
    })
}

pub fn create(
    db: &Database,
    zone: &TimeZone,
    now: Timestamp,
    agent_id: &str,
    input: RoutineInput,
) -> Result<Routine, RoutineError> {
    let name = validate::name(&input.name)?;
    let task = validate::task(&input.task)?;
    let schedule = validate::schedule(&input.schedule)?;
    check_not_past(&schedule, now, zone)?;
    db.with_connection(|connection| {
        if !store::agent_exists(connection, agent_id).map_err(RoutineError::database)? {
            return Err(RoutineError::agent_not_found());
        }
        if store::count_for_agent(connection, agent_id).map_err(RoutineError::database)?
            >= ROUTINES_PER_AGENT
        {
            return Err(RoutineError::new(
                "too-many-routines",
                format!("An agent can have at most {ROUTINES_PER_AGENT} routines."),
            ));
        }
        let created = now.as_millisecond();
        let stored = Stored {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: agent_id.to_string(),
            name,
            task,
            next_run_at: next_run(&schedule, true, false, now, zone),
            schedule,
            active: true,
            done: false,
            created_at: created,
            updated_at: created,
        };
        store::insert(connection, &stored, &validate::to_dto(&stored.schedule))?;
        Ok(view(stored, zone))
    })
}

pub fn update(
    db: &Database,
    zone: &TimeZone,
    now: Timestamp,
    routine_id: &str,
    input: RoutineInput,
) -> Result<Routine, RoutineError> {
    let name = validate::name(&input.name)?;
    let task = validate::task(&input.task)?;
    let schedule = validate::schedule(&input.schedule)?;
    db.with_connection(|connection| {
        let existing = store::get(connection, routine_id)?.ok_or_else(RoutineError::not_found)?;
        let changed = existing.schedule != schedule;
        if changed {
            check_not_past(&schedule, now, zone)?;
        }
        let done = existing.done && !changed;
        let stored = Stored {
            name,
            task,
            next_run_at: next_run(&schedule, existing.active, done, now, zone),
            schedule,
            done,
            updated_at: now.as_millisecond(),
            ..existing
        };
        store::update(connection, &stored, &validate::to_dto(&stored.schedule))?;
        with_last(connection, view(stored, zone), zone, now)
    })
}

pub fn set_active(
    db: &Database,
    zone: &TimeZone,
    now: Timestamp,
    routine_id: &str,
    active: bool,
) -> Result<Routine, RoutineError> {
    db.with_connection(|connection| {
        let existing = store::get(connection, routine_id)?.ok_or_else(RoutineError::not_found)?;
        let stored = Stored {
            active,
            next_run_at: next_run(&existing.schedule, active, existing.done, now, zone),
            updated_at: now.as_millisecond(),
            ..existing
        };
        store::set_active(
            connection,
            &stored.id,
            stored.active,
            stored.next_run_at,
            stored.updated_at,
        )?;
        with_last(connection, view(stored, zone), zone, now)
    })
}

pub fn delete(db: &Database, routine_id: &str) -> Result<(), RoutineError> {
    db.with_connection(|connection| match store::delete(connection, routine_id)? {
        0 => Err(RoutineError::not_found()),
        _ => Ok(()),
    })
}

pub fn refresh_next_runs(
    db: &Database,
    zone: &TimeZone,
    now: Timestamp,
) -> Result<(), RoutineError> {
    db.with_connection(|connection: &Connection| {
        for routine in store::list_all(connection)? {
            if routine
                .next_run_at
                .is_some_and(|at| at <= now.as_millisecond())
            {
                continue;
            }
            let next = next_run(&routine.schedule, routine.active, routine.done, now, zone);
            if next != routine.next_run_at {
                store::set_next_run(connection, &routine.id, next)?;
            }
        }
        Ok(())
    })
}

fn unavailable() -> RoutineError {
    RoutineError::new("unavailable", "The app database is not available.")
}

#[tauri::command]
pub async fn list_routines(app: AppHandle, agent_id: String) -> Result<Vec<Routine>, RoutineError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        list(&db, &zone(), clock::now(), &agent_id)
    })
    .await
    .map_err(RoutineError::database)?
}

#[tauri::command]
pub async fn create_routine(
    app: AppHandle,
    agent_id: String,
    input: RoutineInput,
) -> Result<Routine, RoutineError> {
    let wake_app = app.clone();
    let routine = tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        create(&db, &zone(), clock::now(), &agent_id, input)
    })
    .await
    .map_err(RoutineError::database)??;
    scheduler::wake(&wake_app);
    Ok(routine)
}

#[tauri::command]
pub async fn update_routine(
    app: AppHandle,
    routine_id: String,
    input: RoutineInput,
) -> Result<Routine, RoutineError> {
    let wake_app = app.clone();
    let routine = tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        update(&db, &zone(), clock::now(), &routine_id, input)
    })
    .await
    .map_err(RoutineError::database)??;
    scheduler::wake(&wake_app);
    Ok(routine)
}

#[tauri::command]
pub async fn set_routine_active(
    app: AppHandle,
    routine_id: String,
    active: bool,
) -> Result<Routine, RoutineError> {
    let wake_app = app.clone();
    let routine = tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        set_active(&db, &zone(), clock::now(), &routine_id, active)
    })
    .await
    .map_err(RoutineError::database)??;
    scheduler::wake(&wake_app);
    Ok(routine)
}

#[tauri::command]
pub async fn delete_routine(app: AppHandle, routine_id: String) -> Result<(), RoutineError> {
    let delete_app = app.clone();
    let agent_id = tauri::async_runtime::spawn_blocking(move || {
        let db = delete_app.try_state::<Database>().ok_or_else(unavailable)?;
        let agent_id = db.with_connection(|connection| {
            store::get(connection, &routine_id).map(|found| found.map(|routine| routine.agent_id))
        })?;
        delete(&db, &routine_id)?;
        Ok::<_, RoutineError>(agent_id)
    })
    .await
    .map_err(RoutineError::database)??;
    if let Some(agent_id) = agent_id {
        turns::emit_queue_changed(&app, &agent_id);
    }
    scheduler::wake(&app);
    Ok(())
}

#[tauri::command]
pub async fn list_routine_runs(
    app: AppHandle,
    routine_id: String,
) -> Result<Vec<RunView>, RoutineError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        history(&db, &zone(), clock::now(), &routine_id)
    })
    .await
    .map_err(RoutineError::database)?
}

#[tauri::command]
pub async fn run_routine_now(
    app: AppHandle,
    routine_id: String,
) -> Result<Option<RunView>, RoutineError> {
    let load_app = app.clone();
    let lookup = routine_id.clone();
    let stored = tauri::async_runtime::spawn_blocking(move || {
        let db = load_app.try_state::<Database>().ok_or_else(unavailable)?;
        let registry = load_app
            .try_state::<TurnRegistry>()
            .ok_or_else(unavailable)?;
        if registry.is_closing() {
            return Err(RoutineError::new("shutting-down", "Shikigami is closing."));
        }
        db.with_connection(|connection| store::get(connection, &lookup))?
            .ok_or_else(RoutineError::not_found)
    })
    .await
    .map_err(RoutineError::database)??;
    let now = clock::now();
    let mark = Mark::new(
        &stored.id,
        &stored.name,
        RunKind::Manual,
        None,
        now.as_millisecond(),
    );
    let run_id = mark.run_id.clone();
    turns::start_routine(app.clone(), stored.agent_id, stored.task, mark).await;
    scheduler::wake(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.try_state::<Database>().ok_or_else(unavailable)?;
        db.with_connection(|connection| runs::get(connection, &run_id, &zone(), now))
            .map_err(RoutineError::database)
    })
    .await
    .map_err(RoutineError::database)?
}

#[tauri::command]
pub fn routine_time_zone() -> String {
    zone_name(&zone())
}

#[cfg(test)]
mod tests;
