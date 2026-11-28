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
    parent_turn_id: Option<String>,
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
            "SELECT group_id, agent_id, root_message_id, hop, parent_turn_id, access
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
        row.and_then(|(group_id, agent_id, root, hop, parent_turn_id, access)| {
            Some(Finished {
                group_id: group_id?,
                agent_id: agent_id?,
                root: root.unwrap_or_else(|| turn_id.to_owned()),
                hop,
                parent_turn_id,
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
            parent_turn_id: Some(turn_id.to_owned()),
            owes_turn_id: None,
            access: turn.access,
            notice: None,
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
    } else {
        let answer = Answer {
            agent_id: Some(turn.agent_id.clone()),
            name: name_of(connection, &turn.agent_id)?,
            finished: true,
            notice: None,
        };
        settle_for(connection, &turn, turn_id, answer, now, &mut notices)?;
    }
    Ok(notices)
}

pub fn unfinished_after(
    connection: &Connection,
    turn_id: &str,
    now: i64,
) -> rusqlite::Result<Vec<Notice>> {
    let mut notices = Vec::new();
    let Some(turn) = finished_turn(connection, turn_id)? else {
        return Ok(notices);
    };
    let name = name_of(connection, &turn.agent_id)?;
    let answer = Answer {
        agent_id: Some(turn.agent_id.clone()),
        notice: Some(format!(
            "{name} did not finish; continuing without its answer."
        )),
        name,
        finished: false,
    };
    settle_for(connection, &turn, turn_id, answer, now, &mut notices)?;
    Ok(notices)
}

pub fn settle_interrupted(
    connection: &Connection,
    turn_ids: &[String],
    now: i64,
) -> rusqlite::Result<()> {
    for turn_id in turn_ids {
        unfinished_after(connection, turn_id, now)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Dropped {
    Departed,
    Removed,
}

pub fn drop_waiting(
    connection: &Connection,
    queued_id: &str,
    why: Dropped,
    now: i64,
) -> rusqlite::Result<Option<String>> {
    type Row = (String, String, Option<String>, String, i64, i64);
    let row: Option<Row> = connection
        .query_row(
            "DELETE FROM group_queue WHERE id = ?1
             RETURNING group_id, kind, parent_turn_id, addressed_agent_id, hop, access",
            [queued_id],
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
    let Some((group_id, kind, parent, agent_id, hop, access)) = row else {
        return Ok(None);
    };
    if EntryKind::parse(&kind) == EntryKind::User {
        return Ok(Some(group_id));
    }
    let name = name_of(connection, &agent_id)?;
    let notice = match why {
        Dropped::Departed => format!("{name} did not finish; continuing without its answer."),
        Dropped::Removed => format!("{name}'s task was removed; continuing without its answer."),
    };
    let answer = Answer {
        agent_id: Some(agent_id),
        name,
        finished: false,
        notice: Some(notice),
    };
    let settled = settle(
        connection,
        &group_id,
        parent.clone(),
        answer,
        hop,
        AccessLevel::from_value(access),
        now,
    )?;
    if matches!(settled, Settled::LimitReached) {
        if let Some(parent_id) = parent {
            if let Some(turn) = finished_turn(connection, &parent_id)? {
                post_limit_notice(connection, &turn, &parent_id, now, &mut Vec::new())?;
            }
        }
    }
    Ok(Some(group_id))
}

pub fn drop_for_departed(
    connection: &Connection,
    group_id: Option<&str>,
    agent_id: &str,
    now: i64,
) -> rusqlite::Result<()> {
    if group_id.is_none() {
        connection.execute("DELETE FROM group_members WHERE agent_id = ?1", [agent_id])?;
    }
    let waiting: Vec<String> = {
        let mut statement = connection.prepare(
            "SELECT id FROM group_queue
             WHERE addressed_agent_id = ?1 AND kind IN ('hand-off', 'continuation')
               AND (?2 IS NULL OR group_id = ?2)
             ORDER BY position",
        )?;
        let rows = statement.query_map(params![agent_id, group_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for id in waiting {
        drop_waiting(connection, &id, Dropped::Departed, now)?;
    }
    Ok(())
}

struct Answer {
    agent_id: Option<String>,
    name: String,
    finished: bool,
    notice: Option<String>,
}

enum Settled {
    Nothing,
    Queued,
    LimitReached,
}

fn settle_for(
    connection: &Connection,
    turn: &Finished,
    turn_id: &str,
    answer: Answer,
    now: i64,
    notices: &mut Vec<Notice>,
) -> rusqlite::Result<()> {
    let settled = settle(
        connection,
        &turn.group_id,
        turn.parent_turn_id.clone(),
        answer,
        turn.hop,
        turn.access,
        now,
    )?;
    if matches!(settled, Settled::LimitReached) {
        post_limit_notice(connection, turn, turn_id, now, notices)?;
    }
    Ok(())
}

fn settle(
    connection: &Connection,
    group_id: &str,
    first_parent: Option<String>,
    first_answer: Answer,
    hop: i64,
    access: AccessLevel,
    now: i64,
) -> rusqlite::Result<Settled> {
    let mut parent = first_parent;
    let mut answer = first_answer;
    let mut carried: Vec<String> = Vec::new();
    while let Some(parent_id) = parent.take() {
        type Row = (
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
            Option<String>,
        );
        let row: Option<Row> = connection
            .query_row(
                "SELECT agent_id, root_message_id, parent_turn_id, deferred, pending_notice
                 FROM turns WHERE id = ?1",
                [&parent_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((agent_id, root, grandparent, deferred, pending)) = row else {
            break;
        };
        if !deferred {
            break;
        }
        let open: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_queue WHERE parent_turn_id = ?1)
                 OR EXISTS(SELECT 1 FROM turns
                           WHERE parent_turn_id = ?1 AND (status = 'running' OR deferred = 1))",
            [&parent_id],
            |row| row.get(0),
        )?;
        let mut notices: Vec<String> = pending
            .iter()
            .flat_map(|text| text.split('\n'))
            .map(str::to_owned)
            .collect();
        notices.append(&mut carried);
        notices.extend(answer.notice.clone());
        if open {
            let joined = (!notices.is_empty()).then(|| notices.join("\n"));
            connection.execute(
                "UPDATE turns SET pending_notice = ?2 WHERE id = ?1",
                params![parent_id, joined],
            )?;
            break;
        }
        connection.execute(
            "UPDATE turns SET deferred = 0, pending_notice = NULL WHERE id = ?1",
            [&parent_id],
        )?;
        let root = root.unwrap_or_else(|| parent_id.clone());
        let present: bool = match &agent_id {
            Some(agent) => connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = ?1 AND agent_id = ?2)",
                params![group_id, agent],
                |row| row.get(0),
            )?,
            None => false,
        };
        if let (true, Some(agent)) = (present, agent_id.as_deref()) {
            if turns_of_root(connection, group_id, &root)? >= MAX_TURNS {
                return Ok(Settled::LimitReached);
            }
            let text = if answer.finished {
                format!("Continue with {}'s answer.", answer.name)
            } else {
                format!("Continue without {}'s answer.", answer.name)
            };
            let origin = Origin {
                kind: EntryKind::Continuation,
                from_agent_id: answer.agent_id.clone(),
                root: Some(root),
                hop: hop + 1,
                parent_turn_id: grandparent,
                owes_turn_id: Some(parent_id),
                access,
                notice: (!notices.is_empty()).then(|| notices.join("\n")),
            };
            let position = place_at_head(connection, group_id, 0)?;
            enqueue_at(connection, group_id, agent, &text, &origin, position, now)?;
            return Ok(Settled::Queued);
        }
        carried = notices;
        let name = match &agent_id {
            Some(agent) => name_of(connection, agent)?,
            None => "Former member".to_owned(),
        };
        answer = Answer {
            agent_id: None,
            notice: Some(format!(
                "{name} did not finish; continuing without its answer."
            )),
            name,
            finished: false,
        };
        parent = grandparent;
    }
    Ok(Settled::Nothing)
}
