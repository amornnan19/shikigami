use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

use super::{Agent, AgentError};

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
            "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                agent.id,
                agent.name,
                agent.color,
                agent.provider,
                agent.model,
                agent.workspace_path,
                agent.created_at,
                agent.last_activity_at,
            ],
        )
        .map(|_| ())
        .map_err(|error| {
            if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
                AgentError::name_taken()
            } else {
                AgentError::database(error)
            }
        })
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
                (SELECT t.status FROM turns t WHERE t.agent_id = a.id
                   ORDER BY t.started_at DESC, t.rowid DESC LIMIT 1),
                (SELECT t.error FROM turns t WHERE t.agent_id = a.id
                   ORDER BY t.started_at DESC, t.rowid DESC LIMIT 1)
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
        })
    })?;
    rows.collect()
}
