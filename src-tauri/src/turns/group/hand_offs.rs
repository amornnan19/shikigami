use rusqlite::{params, Connection, OptionalExtension};

use super::super::store::{self, NewMessage};
use super::{enqueue_at, AccessLevel, EntryKind, Origin};
use crate::groups::{self, mentions};

pub const MAX_TURNS: i64 = 8;
pub const LIMIT_NOTICE: &str = "Hand-off limit reached. Mention an agent to continue.";
const PING_PONG_RUN: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: String,
    pub text: String,
}

pub const STOPPED_NOTICE: &str = "Stopped. Hand-offs from this message were dropped.";

struct Finished {
    group_id: String,
    agent_id: String,
    root: String,
    hop: i64,
    return_to: Option<String>,
    access: AccessLevel,
}

fn finished_turn(connection: &Connection, turn_id: &str) -> rusqlite::Result<Option<Finished>> {
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
        i64,
    );
    let row: Option<Row> = connection
        .query_row(
            "SELECT group_id, agent_id, root_message_id, hop, return_to, access
             FROM turns WHERE id = ?1",
            [turn_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    Ok(
        row.and_then(|(group_id, agent_id, root, hop, return_to, access)| {
            Some(Finished {
                group_id: group_id?,
                agent_id: agent_id?,
                root: root.unwrap_or_else(|| turn_id.to_owned()),
                hop,
                return_to,
                access: AccessLevel::from_value(access),
            })
        }),
    )
}

fn final_reply(connection: &Connection, turn_id: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT content FROM messages
             WHERE turn_id = ?1 AND role = 'assistant' AND content <> ''
             ORDER BY seq DESC LIMIT 1",
            [turn_id],
            |row| row.get(0),
        )
        .optional()
}

fn turns_of_root(connection: &Connection, group_id: &str, root: &str) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT (SELECT COUNT(*) FROM turns WHERE group_id = ?1 AND root_message_id = ?2)
              + (SELECT COUNT(*) FROM group_queue WHERE group_id = ?1 AND root_message_id = ?2)",
        params![group_id, root],
        |row| row.get(0),
    )
}

fn hand_off_history(
    connection: &Connection,
    group_id: &str,
    root: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut history = Vec::new();
    for sql in [
        "SELECT from_agent_id, agent_id FROM turns
         WHERE group_id = ?1 AND root_message_id = ?2 AND kind = 'hand-off'
           AND from_agent_id IS NOT NULL AND agent_id IS NOT NULL
         ORDER BY rowid",
        "SELECT from_agent_id, addressed_agent_id FROM group_queue
         WHERE group_id = ?1 AND root_message_id = ?2 AND kind = 'hand-off'
           AND from_agent_id IS NOT NULL
         ORDER BY position",
    ] {
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map(params![group_id, root], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            history.push(row?);
        }
    }
    Ok(history)
}

fn same_pair(pair: &(String, String), first: &str, second: &str) -> bool {
    (pair.0 == first && pair.1 == second) || (pair.0 == second && pair.1 == first)
}

fn is_ping_pong(history: &[(String, String)], from: &str, to: &str) -> bool {
    let run = history
        .iter()
        .rev()
        .take_while(|pair| same_pair(pair, from, to))
        .count();
    run + 1 >= PING_PONG_RUN
}

fn place_at_head(connection: &Connection, group_id: &str, offset: i64) -> rusqlite::Result<i64> {
    let head: i64 = connection.query_row(
        "SELECT COALESCE(MIN(position), 0) FROM group_queue WHERE group_id = ?1",
        [group_id],
        |row| row.get(0),
    )?;
    let at = head + offset;
    connection.execute(
        "UPDATE group_queue SET position = -(position + 2) WHERE group_id = ?1 AND position >= ?2",
        params![group_id, at],
    )?;
    connection.execute(
        "UPDATE group_queue SET position = -position - 1 WHERE group_id = ?1 AND position < 0",
        [group_id],
    )?;
    Ok(at)
}

fn name_of(connection: &Connection, agent_id: &str) -> rusqlite::Result<String> {
    connection
        .query_row("SELECT name FROM agents WHERE id = ?1", [agent_id], |row| {
            row.get(0)
        })
        .optional()
        .map(|name| name.unwrap_or_else(|| "Former member".to_owned()))
}

fn post_notice(
    connection: &Connection,
    turn: &Finished,
    turn_id: &str,
    text: &str,
    now: i64,
) -> rusqlite::Result<Notice> {
    let id = uuid::Uuid::now_v7().to_string();
    store::insert_message_in(
        connection,
        &NewMessage {
            id: &id,
            agent_id: &turn.agent_id,
            turn_id,
            group_id: Some(&turn.group_id),
            role: "notice",
            content: text,
            created_at: now,
        },
    )?;
    Ok(Notice {
        id,
        text: text.to_owned(),
    })
}

fn post_limit_notice(
    connection: &Connection,
    turn: &Finished,
    turn_id: &str,
    now: i64,
    notices: &mut Vec<Notice>,
) -> rusqlite::Result<()> {
    let posted: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages m JOIN turns t ON t.id = m.turn_id
                       WHERE t.group_id = ?1 AND t.root_message_id = ?2
                         AND m.role = 'notice' AND m.content = ?3)",
        params![turn.group_id, turn.root, LIMIT_NOTICE],
        |row| row.get(0),
    )?;
    if !posted {
        notices.push(post_notice(connection, turn, turn_id, LIMIT_NOTICE, now)?);
    }
    Ok(())
}

pub fn drop_after_stop(
    connection: &Connection,
    turn_id: &str,
    now: i64,
) -> rusqlite::Result<Vec<Notice>> {
    let Some(turn) = finished_turn(connection, turn_id)? else {
        return Ok(Vec::new());
    };
    let dropped = connection.execute(
        "DELETE FROM group_queue
         WHERE group_id = ?1 AND root_message_id = ?2 AND kind IN ('hand-off', 'continuation')",
        params![turn.group_id, turn.root],
    )?;
    if dropped == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![post_notice(
        connection,
        &turn,
        turn_id,
        STOPPED_NOTICE,
        now,
    )?])
}

pub fn queue_after(
    connection: &Connection,
    turn_id: &str,
    now: i64,
) -> rusqlite::Result<Vec<Notice>> {
    let mut notices = Vec::new();
    let Some(turn) = finished_turn(connection, turn_id)? else {
        return Ok(notices);
    };
    let members = groups::store::members_of(connection, &turn.group_id)?;
    let reply = final_reply(connection, turn_id)?.unwrap_or_default();
    let mut created = 0;
    for hand_off in mentions::hand_offs(&reply, &members, &turn.agent_id) {
        if turns_of_root(connection, &turn.group_id, &turn.root)? >= MAX_TURNS {
            post_limit_notice(connection, &turn, turn_id, now, &mut notices)?;
            break;
        }
        let history = hand_off_history(connection, &turn.group_id, &turn.root)?;
        if is_ping_pong(&history, &turn.agent_id, &hand_off.agent_id) {
            let text = format!(
                "{} and {} keep handing work back; that hand-off was not started.",
                name_of(connection, &turn.agent_id)?,
                name_of(connection, &hand_off.agent_id)?
            );
            notices.push(post_notice(connection, &turn, turn_id, &text, now)?);
            continue;
        }
        let origin = Origin {
            kind: EntryKind::HandOff,
            from_agent_id: Some(turn.agent_id.clone()),
            root: Some(turn.root.clone()),
            hop: turn.hop + 1,
            return_to: Some(turn.agent_id.clone()),
            parent_turn_id: Some(turn_id.to_owned()),
            access: turn.access,
            truncated: false,
        };
        let position = place_at_head(connection, &turn.group_id, created)?;
        enqueue_at(
            connection,
            &turn.group_id,
            &hand_off.agent_id,
            &hand_off.task,
            &origin,
            position,
            now,
        )?;
        created += 1;
    }
    if created > 0 {
        connection.execute("UPDATE turns SET deferred = 1 WHERE id = ?1", [turn_id])?;
    } else if let Some(owed) = turn.return_to.as_deref() {
        queue_continuation(connection, &turn, turn_id, owed, now, &mut notices)?;
    }
    Ok(notices)
}

fn owed_elsewhere(connection: &Connection, turn: &Finished, owed: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_queue
                       WHERE group_id = ?1 AND root_message_id = ?2 AND return_to = ?3)
             OR EXISTS(SELECT 1 FROM turns
                       WHERE group_id = ?1 AND root_message_id = ?2 AND return_to = ?3
                         AND (status = 'running' OR deferred = 1))",
        params![turn.group_id, turn.root, owed],
        |row| row.get(0),
    )
}

fn queue_continuation(
    connection: &Connection,
    turn: &Finished,
    turn_id: &str,
    owed: &str,
    now: i64,
    notices: &mut Vec<Notice>,
) -> rusqlite::Result<()> {
    let still_member: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = ?1 AND agent_id = ?2)",
        params![turn.group_id, owed],
        |row| row.get(0),
    )?;
    let waiting: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_queue
                       WHERE group_id = ?1 AND root_message_id = ?2 AND addressed_agent_id = ?3
                         AND kind = 'continuation')",
        params![turn.group_id, turn.root, owed],
        |row| row.get(0),
    )?;
    if !still_member || waiting || owed_elsewhere(connection, turn, owed)? {
        return Ok(());
    }
    let debt: Option<(String, Option<String>)> = connection
        .query_row(
            "SELECT id, return_to FROM turns
             WHERE group_id = ?1 AND root_message_id = ?2 AND agent_id = ?3 AND deferred = 1
             ORDER BY rowid DESC LIMIT 1",
            params![turn.group_id, turn.root, owed],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((debt_id, carried)) = debt else {
        return Ok(());
    };
    if turns_of_root(connection, &turn.group_id, &turn.root)? >= MAX_TURNS {
        return post_limit_notice(connection, turn, turn_id, now, notices);
    }
    connection.execute("UPDATE turns SET deferred = 0 WHERE id = ?1", [debt_id])?;
    let text = format!(
        "Continue with {}'s answer.",
        name_of(connection, &turn.agent_id)?
    );
    let origin = Origin {
        kind: EntryKind::Continuation,
        from_agent_id: Some(turn.agent_id.clone()),
        root: Some(turn.root.clone()),
        hop: turn.hop + 1,
        return_to: carried,
        parent_turn_id: Some(turn_id.to_owned()),
        access: turn.access,
        truncated: false,
    };
    let position = place_at_head(connection, &turn.group_id, 0)?;
    enqueue_at(
        connection,
        &turn.group_id,
        owed,
        &text,
        &origin,
        position,
        now,
    )?;
    Ok(())
}
