use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::claude_code::Session;
use super::store::{self, BeginError, Begun, QueuedMessage};
use crate::groups::{self, Member};
use crate::providers::CLAUDE_CODE_ID;

pub struct GroupBegun {
    pub begun: Begun,
    pub agent_id: String,
    pub group_id: String,
    pub folder: String,
    pub group_name: String,
    pub members: Vec<Member>,
    pub lead_agent_id: Option<String>,
    pub input: String,
    pub previous_seen: i64,
    pub stored_context_hash: Option<String>,
}

pub enum GroupSent {
    Started(GroupBegun),
    Queued {
        queued: QueuedMessage,
        started: Option<GroupBegun>,
    },
}

struct GroupFacts {
    name: String,
    folder: String,
    lead_agent_id: Option<String>,
    members: Vec<Member>,
}

fn facts(connection: &Connection, group_id: &str) -> Result<GroupFacts, BeginError> {
    let row = groups::store::find_row(connection, group_id)?.ok_or(BeginError::NotFound)?;
    let members = groups::store::members_of(connection, group_id)?;
    if let Some(attention) =
        groups::derive_attention(Path::new(&row.folder_path).is_dir(), members.len())
    {
        return Err(BeginError::Refused {
            code: attention.code,
            message: attention.message,
        });
    }
    Ok(GroupFacts {
        name: row.name,
        folder: row.folder_path,
        lead_agent_id: row.lead_agent_id,
        members,
    })
}

pub fn lead_of(connection: &Connection, group_id: &str) -> Result<String, BeginError> {
    groups::store::find_row(connection, group_id)?
        .ok_or(BeginError::NotFound)?
        .lead_agent_id
        .ok_or(BeginError::Refused {
            code: "no-lead",
            message: "This group has no lead.".to_owned(),
        })
}

fn busy(connection: &Connection, group_id: &str, agent_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns WHERE status = 'running'
                       AND (group_id = ?1 OR agent_id = ?2))",
        params![group_id, agent_id],
        |row| row.get(0),
    )
}

fn session_of(
    connection: &Connection,
    agent_id: &str,
    group_id: &str,
) -> rusqlite::Result<(Session, i64, Option<String>)> {
    connection.execute(
        "INSERT OR IGNORE INTO agent_group_sessions (agent_id, group_id) VALUES (?1, ?2)",
        params![agent_id, group_id],
    )?;
    let (stored, last_seen, hash): (Option<String>, i64, Option<String>) = connection.query_row(
        "SELECT provider_session_id, last_seen_seq, context_hash FROM agent_group_sessions
         WHERE agent_id = ?1 AND group_id = ?2",
        params![agent_id, group_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let session = match stored {
        Some(id) => Session::Resume(id),
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            connection.execute(
                "UPDATE agent_group_sessions SET provider_session_id = ?3
                 WHERE agent_id = ?1 AND group_id = ?2",
                params![agent_id, group_id, id],
            )?;
            Session::New(id)
        }
    };
    Ok((session, last_seen, hash))
}

fn unseen_lines(
    connection: &Connection,
    group_id: &str,
    agent_id: &str,
    after_rowid: i64,
) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT m.role, m.content, a.name FROM messages m
         LEFT JOIN agents a ON a.id = m.author_agent_id
         WHERE m.group_id = ?1 AND m.seq > ?3 AND m.role IN ('user', 'assistant')
           AND m.author_agent_id IS NOT ?2
           AND m.content <> ''
         ORDER BY m.seq",
    )?;
    let rows = statement.query_map(params![group_id, agent_id, after_rowid], |row| {
        let role: String = row.get(0)?;
        let content: String = row.get(1)?;
        let name: Option<String> = row.get(2)?;
        let author = match (role.as_str(), name) {
            ("user", _) => "User".to_owned(),
            (_, Some(name)) => name,
            (_, None) => "Former member".to_owned(),
        };
        Ok(format!("{author}: {content}"))
    })?;
    rows.collect()
}

pub fn compose_input(unseen: &[String], text: &str) -> String {
    if unseen.is_empty() {
        text.to_owned()
    } else {
        format!("{}\n\n{text}", unseen.join("\n"))
    }
}

fn begin_in(
    transaction: &Connection,
    group_id: &str,
    addressed: &str,
    text: &str,
    now: i64,
) -> Result<GroupBegun, BeginError> {
    let facts = facts(transaction, group_id)?;
    if !facts
        .members
        .iter()
        .any(|member| member.agent_id == addressed)
    {
        return Err(BeginError::Refused {
            code: "not-member",
            message: "This agent is no longer in the group.".to_owned(),
        });
    }
    let agent = store::agent(transaction, addressed)?.ok_or(BeginError::NotFound)?;
    if agent.provider != CLAUDE_CODE_ID {
        return Err(BeginError::Refused {
            code: "provider-unavailable",
            message: "This agent's provider is not available.".to_owned(),
        });
    }
    if busy(transaction, group_id, addressed)? {
        return Err(BeginError::Busy);
    }
    let (session, last_seen, stored_context_hash) = session_of(transaction, addressed, group_id)?;
    let unseen = unseen_lines(transaction, group_id, addressed, last_seen)?;
    let turn_id = uuid::Uuid::now_v7().to_string();
    transaction.execute(
        "INSERT INTO turns (id, agent_id, group_id, status, started_at)
         VALUES (?1, ?2, ?3, 'running', ?4)",
        params![turn_id, addressed, group_id, now],
    )?;
    transaction.execute(
        "INSERT INTO messages (id, agent_id, turn_id, group_id, role, content, created_at)
         VALUES (?1, ?2, ?3, ?4, 'user', ?5, ?6)",
        params![
            uuid::Uuid::now_v7().to_string(),
            addressed,
            turn_id,
            group_id,
            text,
            now
        ],
    )?;
    let message_seq: i64 = transaction.query_row(
        "SELECT seq FROM messages WHERE rowid = last_insert_rowid()",
        [],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE agent_group_sessions SET last_seen_seq = MAX(last_seen_seq, ?3)
         WHERE agent_id = ?1 AND group_id = ?2",
        params![addressed, group_id, message_seq],
    )?;
    transaction.execute(
        "UPDATE agents SET last_activity_at = ?2 WHERE id = ?1",
        params![addressed, now],
    )?;
    transaction.execute(
        "UPDATE groups SET last_activity_at = ?2 WHERE id = ?1",
        params![group_id, now],
    )?;
    Ok(GroupBegun {
        begun: Begun {
            turn_id,
            agent,
            session,
        },
        agent_id: addressed.to_owned(),
        group_id: group_id.to_owned(),
        folder: facts.folder,
        group_name: facts.name,
        members: facts.members,
        lead_agent_id: facts.lead_agent_id,
        input: compose_input(&unseen, text),
        previous_seen: last_seen,
        stored_context_hash,
    })
}

fn enqueue(
    connection: &Connection,
    group_id: &str,
    addressed: &str,
    text: &str,
    now: i64,
) -> rusqlite::Result<QueuedMessage> {
    let id = uuid::Uuid::now_v7().to_string();
    let position: i64 = connection.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM group_queue WHERE group_id = ?1",
        [group_id],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, group_id, addressed, text, position, now],
    )?;
    Ok(QueuedMessage {
        id,
        text: text.to_owned(),
        position,
        created_at: now,
    })
}

pub fn begin_or_queue(
    connection: &Connection,
    group_id: &str,
    text: &str,
    now: i64,
) -> Result<GroupSent, BeginError> {
    let transaction = connection.unchecked_transaction()?;
    facts(&transaction, group_id)?;
    let addressed = lead_of(&transaction, group_id)?;
    let addressed = addressed.as_str();
    let backlog: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_queue WHERE group_id = ?1)",
        [group_id],
        |row| row.get(0),
    )?;
    let sent = if backlog {
        let queued = enqueue(&transaction, group_id, addressed, text, now)?;
        let started = advance_in(&transaction, group_id, now)?;
        GroupSent::Queued { queued, started }
    } else {
        match begin_in(&transaction, group_id, addressed, text, now) {
            Ok(begun) => GroupSent::Started(begun),
            Err(BeginError::Busy) => GroupSent::Queued {
                queued: enqueue(&transaction, group_id, addressed, text, now)?,
                started: None,
            },
            Err(error) => return Err(error),
        }
    };
    transaction.commit()?;
    Ok(sent)
}

pub fn head_agent(connection: &Connection, group_id: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT addressed_agent_id FROM group_queue WHERE group_id = ?1
             ORDER BY position LIMIT 1",
            [group_id],
            |row| row.get(0),
        )
        .optional()
}

pub fn effective_head(connection: &Connection, group_id: &str) -> rusqlite::Result<Option<String>> {
    let Some(head) = head_agent(connection, group_id)? else {
        return Ok(None);
    };
    let member: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = ?1 AND agent_id = ?2)",
        params![group_id, head],
        |row| row.get(0),
    )?;
    if member {
        return Ok(Some(head));
    }
    Ok(groups::store::find_row(connection, group_id)?
        .and_then(|row| row.lead_agent_id)
        .filter(|lead| *lead != head))
}

fn advance_in(
    transaction: &Connection,
    group_id: &str,
    now: i64,
) -> Result<Option<GroupBegun>, BeginError> {
    let head: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT id, addressed_agent_id, content FROM group_queue WHERE group_id = ?1
             ORDER BY position LIMIT 1",
            [group_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((queued_id, mut addressed, text)) = head else {
        return Ok(None);
    };
    let member: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = ?1 AND agent_id = ?2)",
        params![group_id, addressed],
        |row| row.get(0),
    )?;
    if !member {
        let lead = groups::store::find_row(transaction, group_id)?
            .and_then(|row| row.lead_agent_id)
            .filter(|lead| *lead != addressed);
        match lead {
            Some(lead) => addressed = lead,
            None => {
                transaction.execute("DELETE FROM group_queue WHERE id = ?1", [&queued_id])?;
                return Ok(None);
            }
        }
    }
    if busy(transaction, group_id, &addressed)? {
        return Ok(None);
    }
    transaction.execute("DELETE FROM group_queue WHERE id = ?1", [queued_id])?;
    begin_in(transaction, group_id, &addressed, &text, now).map(Some)
}

pub fn advance_queue(
    connection: &Connection,
    group_id: &str,
    now: i64,
) -> Result<Option<GroupBegun>, BeginError> {
    let transaction = connection.unchecked_transaction()?;
    let advanced = advance_in(&transaction, group_id, now)?;
    transaction.commit()?;
    Ok(advanced)
}

pub fn queued_groups_for(connection: &Connection, agent_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT group_id FROM group_queue WHERE addressed_agent_id = ?1
         ORDER BY group_id",
    )?;
    let rows = statement.query_map([agent_id], |row| row.get(0))?;
    rows.collect()
}

pub fn list_queue(connection: &Connection, group_id: &str) -> rusqlite::Result<Vec<QueuedMessage>> {
    let mut statement = connection.prepare(
        "SELECT id, content, position, created_at FROM group_queue
         WHERE group_id = ?1 ORDER BY position",
    )?;
    let rows = statement.query_map([group_id], |row| {
        Ok(QueuedMessage {
            id: row.get(0)?,
            text: row.get(1)?,
            position: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

pub fn edit_queued(
    connection: &Connection,
    queued_id: &str,
    text: &str,
) -> rusqlite::Result<Option<(String, QueuedMessage)>> {
    connection
        .query_row(
            "UPDATE group_queue SET content = ?2 WHERE id = ?1
             RETURNING group_id, id, content, position, created_at",
            params![queued_id, text],
            |row| {
                Ok((
                    row.get(0)?,
                    QueuedMessage {
                        id: row.get(1)?,
                        text: row.get(2)?,
                        position: row.get(3)?,
                        created_at: row.get(4)?,
                    },
                ))
            },
        )
        .optional()
}

pub fn remove_queued(connection: &Connection, queued_id: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "DELETE FROM group_queue WHERE id = ?1 RETURNING group_id",
            [queued_id],
            |row| row.get(0),
        )
        .optional()
}

pub fn set_session(
    connection: &Connection,
    agent_id: &str,
    group_id: &str,
    session_id: Option<&str>,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO agent_group_sessions (agent_id, group_id, provider_session_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (agent_id, group_id) DO UPDATE SET provider_session_id = excluded.provider_session_id",
        params![agent_id, group_id, session_id],
    )?;
    Ok(())
}

pub struct Finish<'a> {
    pub turn_id: &'a str,
    pub agent_id: &'a str,
    pub group_id: &'a str,
    pub status: &'a str,
    pub error: Option<&'a str>,
    pub restore_seen: Option<i64>,
    pub now: i64,
}

pub fn finish_turn(connection: &Connection, finish: &Finish) -> rusqlite::Result<()> {
    let Finish {
        turn_id,
        agent_id,
        group_id,
        status,
        error,
        restore_seen,
        now,
    } = *finish;
    let transaction = connection.unchecked_transaction()?;
    store::finish_turn_rows(&transaction, turn_id, agent_id, status, error, now)?;
    match restore_seen {
        Some(previous) => transaction.execute(
            "UPDATE agent_group_sessions SET last_seen_seq = ?3
             WHERE agent_id = ?1 AND group_id = ?2",
            params![agent_id, group_id, previous],
        )?,
        None => transaction.execute(
            "UPDATE agent_group_sessions
             SET last_seen_seq = MAX(last_seen_seq, COALESCE(
                   (SELECT MAX(seq) FROM messages WHERE turn_id = ?3 AND author_agent_id = ?1), 0))
             WHERE agent_id = ?1 AND group_id = ?2",
            params![agent_id, group_id, turn_id],
        )?,
    };
    transaction.execute(
        "UPDATE groups SET last_activity_at = ?2 WHERE id = ?1",
        params![group_id, now],
    )?;
    transaction.commit()
}

pub fn running_turn_agent(
    connection: &Connection,
    group_id: &str,
    turn_id: &str,
) -> rusqlite::Result<Option<String>> {
    Ok(connection
        .query_row(
            "SELECT agent_id FROM turns WHERE id = ?1 AND group_id = ?2",
            params![turn_id, group_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

pub const FOLDER_INSTRUCTIONS_MAX_BYTES: u64 = 64 * 1024;

pub const MESSAGES_HEADER: &str = "Messages:\n";

pub fn briefing(begun: &GroupBegun) -> String {
    let agent_id = begun.agent_id.as_str();
    let members: Vec<String> = begun
        .members
        .iter()
        .map(|member| {
            let is_you = member.agent_id == agent_id;
            let is_lead = begun.lead_agent_id.as_deref() == Some(member.agent_id.as_str());
            match (is_you, is_lead) {
                (true, true) => format!("{} (you, lead)", member.name),
                (true, false) => format!("{} (you)", member.name),
                (false, true) => format!("{} (lead)", member.name),
                (false, false) => member.name.clone(),
            }
        })
        .collect();
    let name = begun
        .members
        .iter()
        .find(|member| member.agent_id == agent_id)
        .map_or("an agent", |member| member.name.as_str());
    format!(
        "You are {name}, a member of the group chat \"{}\". Members: {}. The user and other members write in this chat. To hand a task to another member, start a line with @Name followed by the task.",
        begun.group_name,
        members.join(", ")
    )
}

pub fn context_block(begun: &GroupBegun, folder_instructions: &str) -> String {
    let mut context = briefing(begun);
    let folder_text = folder_instructions.trim();
    if !folder_text.is_empty() {
        context.push_str("\n\nInstructions from the group folder's CLAUDE.md:\n");
        context.push_str(folder_text);
    }
    context
}

pub fn context_hash(context: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in context.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn set_context_hash(
    connection: &Connection,
    agent_id: &str,
    group_id: &str,
    hash: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE agent_group_sessions SET context_hash = ?3 WHERE agent_id = ?1 AND group_id = ?2",
        params![agent_id, group_id, hash],
    )?;
    Ok(())
}

pub fn restore_seen(
    connection: &Connection,
    agent_id: &str,
    group_id: &str,
    previous: i64,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE agent_group_sessions SET last_seen_seq = ?3 WHERE agent_id = ?1 AND group_id = ?2",
        params![agent_id, group_id, previous],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
