use rusqlite::{params, Connection, OptionalExtension, Row};

use super::schedule::Schedule;
use super::validate;
use super::{RoutineError, ScheduleDto};

pub struct Stored {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub task: String,
    pub schedule: Schedule,
    pub active: bool,
    pub done: bool,
    pub next_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLUMNS: &str = "id, agent_id, name, task, kind, at_time, on_date, days, every_hours, weekday, active, done, next_run_at, created_at, updated_at";

struct Columns {
    kind: &'static str,
    at_time: String,
    on_date: Option<String>,
    days: i64,
    every_hours: Option<i64>,
    weekday: Option<i64>,
}

fn columns_of(dto: &ScheduleDto) -> Columns {
    match dto {
        ScheduleDto::Once { date, time } => Columns {
            kind: "once",
            at_time: time.clone(),
            on_date: Some(date.clone()),
            days: 127,
            every_hours: None,
            weekday: None,
        },
        ScheduleDto::Daily { time, days } => Columns {
            kind: "daily",
            at_time: time.clone(),
            on_date: None,
            days: *days,
            every_hours: None,
            weekday: None,
        },
        ScheduleDto::Hourly {
            time,
            every_hours,
            days,
        } => Columns {
            kind: "hourly",
            at_time: time.clone(),
            on_date: None,
            days: *days,
            every_hours: Some(*every_hours),
            weekday: None,
        },
        ScheduleDto::Weekly { weekday, time } => Columns {
            kind: "weekly",
            at_time: time.clone(),
            on_date: None,
            days: 127,
            every_hours: None,
            weekday: Some(*weekday),
        },
    }
}

fn dto_of(
    kind: &str,
    at_time: String,
    on_date: Option<String>,
    days: i64,
    every_hours: Option<i64>,
    weekday: Option<i64>,
) -> Option<ScheduleDto> {
    match kind {
        "once" => Some(ScheduleDto::Once {
            date: on_date?,
            time: at_time,
        }),
        "daily" => Some(ScheduleDto::Daily {
            time: at_time,
            days,
        }),
        "hourly" => Some(ScheduleDto::Hourly {
            time: at_time,
            every_hours: every_hours?,
            days,
        }),
        "weekly" => Some(ScheduleDto::Weekly {
            weekday: weekday?,
            time: at_time,
        }),
        _ => None,
    }
}

fn read(row: &Row) -> rusqlite::Result<Result<Stored, RoutineError>> {
    let id: String = row.get(0)?;
    let kind: String = row.get(4)?;
    let dto = dto_of(
        &kind,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    );
    let schedule = dto
        .ok_or_else(|| RoutineError::stored(&id))
        .and_then(|dto| validate::schedule(&dto).map_err(|_| RoutineError::stored(&id)));
    Ok(schedule.and_then(|schedule| {
        Ok(Stored {
            agent_id: row.get(1).map_err(RoutineError::database)?,
            name: row.get(2).map_err(RoutineError::database)?,
            task: row.get(3).map_err(RoutineError::database)?,
            schedule,
            active: row.get(10).map_err(RoutineError::database)?,
            done: row.get(11).map_err(RoutineError::database)?,
            next_run_at: row.get(12).map_err(RoutineError::database)?,
            created_at: row.get(13).map_err(RoutineError::database)?,
            updated_at: row.get(14).map_err(RoutineError::database)?,
            id,
        })
    }))
}

pub fn agent_exists(connection: &Connection, agent_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
        [agent_id],
        |row| row.get(0),
    )
}

pub fn count_for_agent(connection: &Connection, agent_id: &str) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COUNT(*) FROM routines WHERE agent_id = ?1",
        [agent_id],
        |row| row.get(0),
    )
}

pub fn get(connection: &Connection, id: &str) -> Result<Option<Stored>, RoutineError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM routines WHERE id = ?1"),
            [id],
            read,
        )
        .optional()
        .map_err(RoutineError::database)?
        .transpose()
}

pub fn list_for_agent(
    connection: &Connection,
    agent_id: &str,
) -> Result<Vec<Stored>, RoutineError> {
    query(
        connection,
        &format!("SELECT {COLUMNS} FROM routines WHERE agent_id = ?1 ORDER BY created_at, id"),
        params![agent_id],
    )
}

pub fn list_due(connection: &Connection, now_ms: i64) -> Result<Vec<Stored>, RoutineError> {
    query(
        connection,
        &format!(
            "SELECT {COLUMNS} FROM routines
             WHERE active = 1 AND done = 0 AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at, name, id"
        ),
        params![now_ms],
    )
}

pub fn earliest_next_run(connection: &Connection) -> rusqlite::Result<Option<i64>> {
    connection.query_row(
        "SELECT MIN(next_run_at) FROM routines WHERE active = 1 AND done = 0",
        [],
        |row| row.get(0),
    )
}

pub fn advance(
    connection: &Connection,
    id: &str,
    next_run_at: Option<i64>,
    done: bool,
) -> Result<(), RoutineError> {
    connection
        .execute(
            "UPDATE routines SET next_run_at = ?2, done = done OR ?3 WHERE id = ?1",
            params![id, next_run_at, done],
        )
        .map(|_| ())
        .map_err(RoutineError::database)
}

pub fn list_all(connection: &Connection) -> Result<Vec<Stored>, RoutineError> {
    query(
        connection,
        &format!("SELECT {COLUMNS} FROM routines ORDER BY created_at, id"),
        [],
    )
}

fn query(
    connection: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<Stored>, RoutineError> {
    let mut statement = connection.prepare(sql).map_err(RoutineError::database)?;
    let rows = statement
        .query_map(parameters, read)
        .map_err(RoutineError::database)?;
    let mut stored = Vec::new();
    for row in rows {
        match row.map_err(RoutineError::database)? {
            Ok(routine) => stored.push(routine),
            Err(_) => continue,
        }
    }
    Ok(stored)
}

pub fn insert(
    connection: &Connection,
    routine: &Stored,
    dto: &ScheduleDto,
) -> Result<(), RoutineError> {
    let columns = columns_of(dto);
    connection
        .execute(
            "INSERT INTO routines (id, agent_id, name, task, kind, at_time, on_date, days, every_hours, weekday, active, done, next_run_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                routine.id,
                routine.agent_id,
                routine.name,
                routine.task,
                columns.kind,
                columns.at_time,
                columns.on_date,
                columns.days,
                columns.every_hours,
                columns.weekday,
                routine.active,
                routine.done,
                routine.next_run_at,
                routine.created_at,
                routine.updated_at,
            ],
        )
        .map(|_| ())
        .map_err(RoutineError::save)
}

pub fn update(
    connection: &Connection,
    routine: &Stored,
    dto: &ScheduleDto,
) -> Result<(), RoutineError> {
    let columns = columns_of(dto);
    connection
        .execute(
            "UPDATE routines SET name = ?2, task = ?3, kind = ?4, at_time = ?5, on_date = ?6, days = ?7, every_hours = ?8, weekday = ?9, done = ?10, next_run_at = ?11, updated_at = ?12
             WHERE id = ?1",
            params![
                routine.id,
                routine.name,
                routine.task,
                columns.kind,
                columns.at_time,
                columns.on_date,
                columns.days,
                columns.every_hours,
                columns.weekday,
                routine.done,
                routine.next_run_at,
                routine.updated_at,
            ],
        )
        .map(|_| ())
        .map_err(RoutineError::save)
}

pub fn set_active(
    connection: &Connection,
    id: &str,
    active: bool,
    next_run_at: Option<i64>,
    updated_at: i64,
) -> Result<(), RoutineError> {
    connection
        .execute(
            "UPDATE routines SET active = ?2, next_run_at = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, active, next_run_at, updated_at],
        )
        .map(|_| ())
        .map_err(RoutineError::database)
}

pub fn set_next_run(
    connection: &Connection,
    id: &str,
    next_run_at: Option<i64>,
) -> Result<(), RoutineError> {
    connection
        .execute(
            "UPDATE routines SET next_run_at = ?2 WHERE id = ?1",
            params![id, next_run_at],
        )
        .map(|_| ())
        .map_err(RoutineError::database)
}

pub fn delete(connection: &Connection, id: &str) -> Result<usize, RoutineError> {
    connection
        .execute("DELETE FROM routines WHERE id = ?1", [id])
        .map_err(RoutineError::database)
}
