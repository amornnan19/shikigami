use rusqlite::{params, Connection, OptionalExtension};

use super::super::store::{self, NewMessage};
use super::{enqueue_at, EntryKind, Origin};
use crate::groups::{self, mentions};

pub const MAX_TURNS: i64 = 8;
pub const LIMIT_NOTICE: &str = "Hand-off limit reached. Mention an agent to continue.";
const PING_PONG_RUN: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: String,
    pub text: String,
}

struct Finished {
    group_id: String,
    agent_id: String,
    root: String,
    hop: i64,
    kind: EntryKind,
    from_agent_id: Option<String>,
}

fn finished_turn(connection: &Connection, turn_id: &str) -> rusqlite::Result<Option<Finished>> {
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        String,
        Option<String>,
    );
    let row: Option<Row> = connection
        .query_row(
            "SELECT group_id, agent_id, root_message_id, hop, kind, from_agent_id
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
        row.and_then(|(group_id, agent_id, root, hop, kind, from_agent_id)| {
            Some(Finished {
                group_id: group_id?,
                agent_id: agent_id?,
                root: root.unwrap_or_else(|| turn_id.to_owned()),
                hop,
                kind: EntryKind::parse(&kind),
                from_agent_id,
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

fn insert_position(connection: &Connection, group_id: &str, root: &str) -> rusqlite::Result<i64> {
    let after: Option<i64> = connection.query_row(
        "SELECT MAX(position) FROM group_queue
         WHERE group_id = ?1 AND root_message_id = ?2 AND kind <> 'user'",
        params![group_id, root],
        |row| row.get(0),
    )?;
    let at = match after {
        Some(last) => last + 1,
        None => connection.query_row(
            "SELECT COALESCE(MIN(position), 0) FROM group_queue WHERE group_id = ?1",
            [group_id],
            |row| row.get(0),
        )?,
    };
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
    let mut limit_reached = false;
    for hand_off in mentions::hand_offs(&reply, &members, &turn.agent_id) {
        if turns_of_root(connection, &turn.group_id, &turn.root)? >= MAX_TURNS {
            post_limit_notice(connection, &turn, turn_id, now, &mut notices)?;
            limit_reached = true;
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
        };
        let position = insert_position(connection, &turn.group_id, &turn.root)?;
        enqueue_at(
            connection,
            &turn.group_id,
            &hand_off.agent_id,
            &hand_off.task,
            &origin,
            position,
            now,
        )?;
    }
    let handed_by = turn
        .from_agent_id
        .as_deref()
        .filter(|_| turn.kind == EntryKind::HandOff && !limit_reached);
    if let Some(from) = handed_by {
        queue_continuation(connection, &turn, from, now, &mut notices, turn_id)?;
    }
    Ok(notices)
}

fn queue_continuation(
    connection: &Connection,
    turn: &Finished,
    from: &str,
    now: i64,
    notices: &mut Vec<Notice>,
    turn_id: &str,
) -> rusqlite::Result<()> {
    let still_member: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = ?1 AND agent_id = ?2)",
        params![turn.group_id, from],
        |row| row.get(0),
    )?;
    let already_waiting: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_queue
                       WHERE group_id = ?1 AND root_message_id = ?2 AND addressed_agent_id = ?3)",
        params![turn.group_id, turn.root, from],
        |row| row.get(0),
    )?;
    if !still_member || already_waiting {
        return Ok(());
    }
    if turns_of_root(connection, &turn.group_id, &turn.root)? >= MAX_TURNS {
        return post_limit_notice(connection, turn, turn_id, now, notices);
    }
    let text = format!(
        "Continue with {}'s answer.",
        name_of(connection, &turn.agent_id)?
    );
    let origin = Origin {
        kind: EntryKind::Continuation,
        from_agent_id: Some(turn.agent_id.clone()),
        root: Some(turn.root.clone()),
        hop: turn.hop + 1,
    };
    let position = insert_position(connection, &turn.group_id, &turn.root)?;
    enqueue_at(
        connection,
        &turn.group_id,
        from,
        &text,
        &origin,
        position,
        now,
    )?;
    Ok(())
}
