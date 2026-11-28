use jiff::tz::TimeZone;
use jiff::Timestamp;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

pub const HISTORY_LIMIT: i64 = 20;

pub const REASON_OVERLAP: &str = "the last run had not finished";
pub const REASON_CLOSED: &str = "Shikigami was not running";
pub const REASON_ASLEEP: &str = "the Mac was asleep";
pub const REASON_REMOVED: &str = "removed from the queue";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Scheduled,
    Manual,
    Late,
}

impl RunKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
            Self::Late => "late",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mark {
    pub run_id: String,
    pub routine_id: String,
    pub routine_name: String,
    pub kind: RunKind,
    pub scheduled_for: Option<i64>,
    pub at: i64,
}

impl Mark {
    pub fn new(
        routine_id: &str,
        routine_name: &str,
        kind: RunKind,
        scheduled_for: Option<i64>,
        at: i64,
    ) -> Self {
        Self {
            run_id: uuid::Uuid::now_v7().to_string(),
            routine_id: routine_id.to_string(),
            routine_name: routine_name.to_string(),
            kind,
            scheduled_for,
            at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: String,
    pub routine_id: String,
    pub scheduled_for: Option<i64>,
    pub turn_id: Option<String>,
    pub kind: String,
    pub outcome: String,
    pub reason: Option<String>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub time_text: String,
}

struct Placement<'a> {
    turn_id: Option<&'a str>,
    queued_id: Option<&'a str>,
    outcome: &'a str,
    reason: Option<&'a str>,
    ended_at: Option<i64>,
}

fn insert(connection: &Connection, mark: &Mark, placement: &Placement) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO routine_runs (id, routine_id, scheduled_for, turn_id, queued_id, kind, outcome, reason, created_at, ended_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            mark.run_id,
            mark.routine_id,
            mark.scheduled_for,
            placement.turn_id,
            placement.queued_id,
            mark.kind.as_str(),
            placement.outcome,
            placement.reason,
            mark.at,
            placement.ended_at,
        ],
    )?;
    connection.execute(
        "DELETE FROM routine_runs
         WHERE routine_id = ?1 AND outcome NOT IN ('queued', 'running')
           AND id NOT IN (SELECT id FROM routine_runs WHERE routine_id = ?1
                          ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
        params![mark.routine_id, HISTORY_LIMIT],
    )?;
    Ok(())
}

pub fn record_running(connection: &Connection, mark: &Mark, turn_id: &str) -> rusqlite::Result<()> {
    insert(
        connection,
        mark,
        &Placement {
            turn_id: Some(turn_id),
            queued_id: None,
            outcome: "running",
            reason: None,
            ended_at: None,
        },
    )
}

pub fn record_queued(
    connection: &Connection,
    mark: &Mark,
    queued_id: &str,
) -> rusqlite::Result<()> {
    insert(
        connection,
        mark,
        &Placement {
            turn_id: None,
            queued_id: Some(queued_id),
            outcome: "queued",
            reason: None,
            ended_at: None,
        },
    )
}

pub fn record_ended(
    connection: &Connection,
    mark: &Mark,
    outcome: &str,
    reason: Option<&str>,
) -> rusqlite::Result<()> {
    insert(
        connection,
        mark,
        &Placement {
            turn_id: None,
            queued_id: None,
            outcome,
            reason,
            ended_at: Some(mark.at),
        },
    )
}

pub fn is_active(connection: &Connection, routine_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM routine_runs
                       WHERE routine_id = ?1 AND outcome IN ('queued', 'running'))",
        [routine_id],
        |row| row.get(0),
    )
}

pub fn link_turn(connection: &Connection, queued_id: &str, turn_id: &str) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE routine_runs SET turn_id = ?2, queued_id = NULL, outcome = 'running'
         WHERE queued_id = ?1",
        params![queued_id, turn_id],
    )?;
    Ok(())
}

pub fn finish_turn(
    connection: &Connection,
    turn_id: &str,
    status: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let outcome = match status {
        "completed" | "stopped" | "interrupted" => status,
        _ => "failed",
    };
    connection.execute(
        "UPDATE routine_runs SET outcome = ?2, ended_at = ?3
         WHERE turn_id = ?1 AND outcome = 'running'",
        params![turn_id, outcome, now],
    )?;
    Ok(())
}

pub fn interrupt_running(connection: &Connection, now: i64) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE routine_runs SET outcome = 'interrupted', ended_at = ?1 WHERE outcome = 'running'",
        [now],
    )?;
    Ok(())
}

pub fn removed_from_queue(
    connection: &Connection,
    queued_id: &str,
    now: i64,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE routine_runs SET outcome = 'skipped', reason = ?3, queued_id = NULL, ended_at = ?2
         WHERE queued_id = ?1",
        params![queued_id, now, REASON_REMOVED],
    )?;
    Ok(())
}

const COLUMNS: &str =
    "id, routine_id, scheduled_for, turn_id, kind, outcome, reason, created_at, ended_at";

fn read(row: &Row, zone: &TimeZone, now: Timestamp) -> rusqlite::Result<RunView> {
    let created_at: i64 = row.get(7)?;
    let ended_at: Option<i64> = row.get(8)?;
    Ok(RunView {
        id: row.get(0)?,
        routine_id: row.get(1)?,
        scheduled_for: row.get(2)?,
        turn_id: row.get(3)?,
        kind: row.get(4)?,
        outcome: row.get(5)?,
        reason: row.get(6)?,
        created_at,
        ended_at,
        time_text: time_text(created_at, zone, now),
    })
}

fn time_text(ms: i64, zone: &TimeZone, now: Timestamp) -> String {
    let at = Timestamp::from_millisecond(ms)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned_datetime(zone.clone());
    let today = now.to_zoned_datetime(zone.clone()).date();
    if at.date() == today {
        at.strftime("%H:%M").to_string()
    } else {
        at.strftime("%-d %b %H:%M").to_string()
    }
}

pub fn list(
    connection: &Connection,
    routine_id: &str,
    zone: &TimeZone,
    now: Timestamp,
) -> rusqlite::Result<Vec<RunView>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM routine_runs WHERE routine_id = ?1
         ORDER BY created_at DESC, rowid DESC LIMIT ?2"
    ))?;
    let rows = statement.query_map(params![routine_id, HISTORY_LIMIT], |row| {
        read(row, zone, now)
    })?;
    rows.collect()
}

pub fn last(
    connection: &Connection,
    routine_id: &str,
    zone: &TimeZone,
    now: Timestamp,
) -> rusqlite::Result<Option<RunView>> {
    connection
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM routine_runs WHERE routine_id = ?1
                 ORDER BY created_at DESC, rowid DESC LIMIT 1"
            ),
            [routine_id],
            |row| read(row, zone, now),
        )
        .optional()
}

pub fn get(
    connection: &Connection,
    run_id: &str,
    zone: &TimeZone,
    now: Timestamp,
) -> rusqlite::Result<Option<RunView>> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM routine_runs WHERE id = ?1"),
            [run_id],
            |row| read(row, zone, now),
        )
        .optional()
}
