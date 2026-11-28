use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use super::{GroupError, Member};

pub struct GroupRow {
    pub id: String,
    pub name: String,
    pub folder_path: String,
    pub lead_agent_id: Option<String>,
    pub created_at: i64,
    pub last_activity_at: i64,
}

const NAME_COLUMN: &str = "groups.name";

pub(super) fn save_error(error: rusqlite::Error) -> GroupError {
    let taken = matches!(
        &error,
        rusqlite::Error::SqliteFailure(failure, Some(message))
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                && message.contains(NAME_COLUMN)
    );
    if taken {
        GroupError::name_taken()
    } else {
        GroupError::database(error)
    }
}

pub fn name_taken_by_other(
    connection: &Connection,
    name: &str,
    except_id: Option<&str>,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE name = ?1 AND id IS NOT ?2)",
        params![name, except_id],
        |row| row.get(0),
    )
}

pub fn agent_exists(connection: &Connection, agent_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
        [agent_id],
        |row| row.get(0),
    )
}

pub fn agent_workspaces(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT workspace_path FROM agents")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn group_folders(
    connection: &Connection,
    except_id: Option<&str>,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut statement =
        connection.prepare("SELECT name, folder_path FROM groups WHERE id IS NOT ?1")?;
    let rows = statement.query_map([except_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

pub fn insert(
    connection: &Connection,
    row: &GroupRow,
    member_ids: &[String],
) -> Result<(), GroupError> {
    connection
        .execute(
            "INSERT INTO groups (id, name, folder_path, lead_agent_id, created_at, last_activity_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                row.id,
                row.name,
                row.folder_path,
                row.lead_agent_id,
                row.created_at,
                row.last_activity_at
            ],
        )
        .map_err(save_error)?;
    for (position, agent_id) in member_ids.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO group_members (group_id, agent_id, position, joined_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![row.id, agent_id, position as i64, row.created_at],
            )
            .map_err(GroupError::database)?;
    }
    Ok(())
}

pub fn find_row(connection: &Connection, group_id: &str) -> rusqlite::Result<Option<GroupRow>> {
    connection
        .query_row(
            "SELECT id, name, folder_path, lead_agent_id, created_at, last_activity_at
             FROM groups WHERE id = ?1",
            [group_id],
            row_of,
        )
        .optional()
}

pub fn member_ids(connection: &Connection, group_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT agent_id FROM group_members WHERE group_id = ?1 ORDER BY position")?;
    let rows = statement.query_map([group_id], |row| row.get(0))?;
    rows.collect()
}

pub fn rename(connection: &Connection, group_id: &str, name: &str) -> Result<(), GroupError> {
    connection
        .execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2",
            params![name, group_id],
        )
        .map(|_| ())
        .map_err(save_error)
}

pub fn set_lead(
    connection: &Connection,
    group_id: &str,
    lead: Option<&str>,
) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE groups SET lead_agent_id = ?1 WHERE id = ?2",
        params![lead, group_id],
    )
}

pub fn replace_members(
    connection: &Connection,
    group_id: &str,
    member_ids_in_order: &[String],
    now: i64,
) -> rusqlite::Result<()> {
    let current = member_ids(connection, group_id)?;
    for agent_id in &current {
        if !member_ids_in_order.contains(agent_id) {
            connection.execute(
                "DELETE FROM group_members WHERE group_id = ?1 AND agent_id = ?2",
                params![group_id, agent_id],
            )?;
        }
    }
    for (position, agent_id) in member_ids_in_order.iter().enumerate() {
        connection.execute(
            "INSERT INTO group_members (group_id, agent_id, position, joined_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (group_id, agent_id) DO UPDATE SET position = excluded.position",
            params![group_id, agent_id, position as i64, now],
        )?;
    }
    Ok(())
}

pub fn has_running_turn(connection: &Connection, group_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns WHERE group_id = ?1 AND status = 'running')",
        [group_id],
        |row| row.get(0),
    )
}

pub fn members_of(connection: &Connection, group_id: &str) -> rusqlite::Result<Vec<Member>> {
    let mut statement = connection.prepare(
        "SELECT a.id, a.name, a.color
         FROM group_members gm JOIN agents a ON a.id = gm.agent_id
         WHERE gm.group_id = ?1
         ORDER BY gm.position, gm.joined_at",
    )?;
    let rows = statement.query_map([group_id], |row| {
        Ok(Member {
            agent_id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn delete(connection: &Connection, group_id: &str) -> rusqlite::Result<usize> {
    connection.execute("DELETE FROM groups WHERE id = ?1", [group_id])
}

pub fn reassign_leads(connection: &Connection, leaving_agent_id: &str) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE groups SET lead_agent_id = (
             SELECT gm.agent_id FROM group_members gm
             WHERE gm.group_id = groups.id AND gm.agent_id <> ?1
             ORDER BY gm.position, gm.joined_at LIMIT 1)
         WHERE lead_agent_id = ?1",
        [leaving_agent_id],
    )
}

fn row_of(row: &rusqlite::Row) -> rusqlite::Result<GroupRow> {
    Ok(GroupRow {
        id: row.get(0)?,
        name: row.get(1)?,
        folder_path: row.get(2)?,
        lead_agent_id: row.get(3)?,
        created_at: row.get(4)?,
        last_activity_at: row.get(5)?,
    })
}

pub fn list(connection: &Connection) -> rusqlite::Result<Vec<(GroupRow, Vec<Member>)>> {
    let mut members: HashMap<String, Vec<Member>> = HashMap::new();
    let mut statement = connection.prepare(
        "SELECT gm.group_id, a.id, a.name, a.color
         FROM group_members gm JOIN agents a ON a.id = gm.agent_id
         ORDER BY gm.position, gm.joined_at",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Member {
                agent_id: row.get(1)?,
                name: row.get(2)?,
                color: row.get(3)?,
            },
        ))
    })?;
    for row in rows {
        let (group_id, member) = row?;
        members.entry(group_id).or_default().push(member);
    }
    let mut statement = connection.prepare(
        "SELECT id, name, folder_path, lead_agent_id, created_at, last_activity_at
         FROM groups
         ORDER BY last_activity_at DESC, created_at DESC, rowid DESC",
    )?;
    let rows = statement.query_map([], row_of)?;
    rows.map(|row| {
        let row = row?;
        let list = members.remove(&row.id).unwrap_or_default();
        Ok((row, list))
    })
    .collect()
}
