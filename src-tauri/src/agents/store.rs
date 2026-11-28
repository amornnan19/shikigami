use rusqlite::{params, Connection, OptionalExtension};

use super::{Agent, AgentError};
use crate::auth::AuthMode;

pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub color: String,
    pub provider: String,
    pub model: String,
    pub workspace_path: String,
    pub created_at: i64,
    pub last_activity_at: i64,
    pub has_running_turn: bool,
    pub newest_turn_status: Option<String>,
    pub newest_turn_error: Option<String>,
    pub auth_mode: Option<String>,
    pub waiting_for_you: bool,
}

const NAME_COLUMN: &str = "agents.name";

fn is_name_taken(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, Some(message))
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                && message.contains(NAME_COLUMN)
    )
}

pub(super) fn save_error(error: rusqlite::Error) -> AgentError {
    if is_name_taken(&error) {
        AgentError::name_taken()
    } else {
        AgentError::database(error)
    }
}

pub fn name_exists(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agents WHERE name = ?1)",
        [name],
        |row| row.get(0),
    )
}

pub fn insert(connection: &Connection, agent: &Agent) -> Result<(), AgentError> {
    connection
        .execute(
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at, auth_mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                agent.id,
                agent.name,
                agent.color,
                agent.provider,
                agent.model,
                agent.workspace_path,
                agent.created_at,
                agent.last_activity_at,
                agent.auth_mode.map(AuthMode::as_str),
            ],
        )
        .map(|_| ())
        .map_err(save_error)
}

pub fn update(
    connection: &Connection,
    agent_id: &str,
    name: Option<&str>,
    color: Option<&str>,
    model: Option<&str>,
) -> Result<usize, AgentError> {
    connection
        .execute(
            "UPDATE agents SET name = COALESCE(?1, name), color = COALESCE(?2, color), model = COALESCE(?3, model)
             WHERE id = ?4",
            params![name, color, model, agent_id],
        )
        .map_err(save_error)
}

pub fn set_auth_mode(
    connection: &Connection,
    agent_id: &str,
    mode: Option<AuthMode>,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE agents SET auth_mode = ?1 WHERE id = ?2",
        params![mode.map(AuthMode::as_str), agent_id],
    )
}

pub fn workspace_paths(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT workspace_path FROM agents")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn path_used_by_other(
    connection: &Connection,
    agent_id: &str,
    path: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agents WHERE workspace_path = ?1 AND id <> ?2)",
        params![path, agent_id],
        |row| row.get(0),
    )
}

pub fn workspace_of(
    connection: &Connection,
    agent_id: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    connection
        .query_row(
            "SELECT name, workspace_path FROM agents WHERE id = ?1",
            [agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
}

pub fn list(connection: &Connection) -> rusqlite::Result<Vec<AgentRow>> {
    let mut statement = connection.prepare(
        "SELECT a.id, a.name, a.color, a.provider, a.model, a.workspace_path, a.created_at, a.last_activity_at,
                EXISTS(SELECT 1 FROM turns t WHERE t.agent_id = a.id AND t.status = 'running'),
                (SELECT t.status FROM turns t WHERE t.agent_id = a.id AND t.group_id IS NULL
                   ORDER BY t.started_at DESC, t.rowid DESC LIMIT 1),
                (SELECT t.error FROM turns t WHERE t.agent_id = a.id AND t.group_id IS NULL
                   ORDER BY t.started_at DESC, t.rowid DESC LIMIT 1),
                a.auth_mode,
                EXISTS(SELECT 1 FROM approvals ap JOIN turns t ON t.id = ap.turn_id
                       WHERE t.agent_id = a.id AND ap.status = 'pending')
         FROM agents a
         ORDER BY a.last_activity_at DESC, a.created_at DESC, a.rowid DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AgentRow {
            id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            workspace_path: row.get(5)?,
            created_at: row.get(6)?,
            last_activity_at: row.get(7)?,
            has_running_turn: row.get(8)?,
            newest_turn_status: row.get(9)?,
            newest_turn_error: row.get(10)?,
            auth_mode: row.get(11)?,
            waiting_for_you: row.get(12)?,
        })
    })?;
    rows.collect()
}
