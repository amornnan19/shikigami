use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::claude_code::Session;

pub const PAGE_SIZE: usize = 20;

pub struct AgentRecord {
    pub provider: String,
    pub model: String,
    pub workspace_path: String,
    pub provider_session_id: Option<String>,
}

pub struct Begun {
    pub turn_id: String,
    pub agent: AgentRecord,
    pub session: Session,
}

#[derive(Debug)]
pub enum BeginError {
    NotFound,
    Busy,
    Refused { code: &'static str, message: String },
    Database(rusqlite::Error),
}

impl From<rusqlite::Error> for BeginError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepView {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ItemView {
    Text { id: String, text: String },
    Notice { id: String, text: String },
    Step(StepView),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandOffView {
    pub agent_id: String,
    pub task: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    pub id: String,
    pub agent_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub kind: String,
    pub from_agent_id: Option<String>,
    pub hand_offs: Vec<HandOffView>,
    pub user_message: String,
    pub items: Vec<ItemView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPage {
    pub turns: Vec<TurnView>,
    pub has_more: bool,
}

pub fn agent(connection: &Connection, agent_id: &str) -> rusqlite::Result<Option<AgentRecord>> {
    connection
        .query_row(
            "SELECT provider, model, workspace_path, provider_session_id FROM agents WHERE id = ?1",
            [agent_id],
            |row| {
                Ok(AgentRecord {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    workspace_path: row.get(2)?,
                    provider_session_id: row.get(3)?,
                })
            },
        )
        .optional()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
    pub position: i64,
    pub created_at: i64,
}

pub enum Sent {
    Started(Begun),
    Queued {
        queued: QueuedMessage,
        started: Option<Advanced>,
    },
}

pub struct Advanced {
    pub begun: Begun,
    pub text: String,
}

#[cfg(test)]
pub fn begin_turn(
    connection: &Connection,
    agent_id: &str,
    text: &str,
    now: i64,
) -> Result<Begun, BeginError> {
    let transaction = connection.unchecked_transaction()?;
    let begun = begin_in(&transaction, agent_id, text, now)?;
    transaction.commit()?;
    Ok(begun)
}

pub fn begin_or_queue(
    connection: &Connection,
    agent_id: &str,
    text: &str,
    now: i64,
) -> Result<Sent, BeginError> {
    let transaction = connection.unchecked_transaction()?;
    let backlog: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM queued_messages WHERE agent_id = ?1)",
        [agent_id],
        |row| row.get(0),
    )?;
    let sent = if backlog {
        let queued = enqueue(&transaction, agent_id, text, now)?;
        let started = advance_in(&transaction, agent_id, now)?;
        Sent::Queued { queued, started }
    } else {
        match begin_in(&transaction, agent_id, text, now) {
            Ok(begun) => Sent::Started(begun),
            Err(BeginError::Busy) => Sent::Queued {
                queued: enqueue(&transaction, agent_id, text, now)?,
                started: None,
            },
            Err(error) => return Err(error),
        }
    };
    transaction.commit()?;
    Ok(sent)
}

fn begin_in(
    transaction: &Connection,
    agent_id: &str,
    text: &str,
    now: i64,
) -> Result<Begun, BeginError> {
    let agent = agent(transaction, agent_id)?.ok_or(BeginError::NotFound)?;
    let running: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns WHERE agent_id = ?1 AND status = 'running')",
        [agent_id],
        |row| row.get(0),
    )?;
    if running {
        return Err(BeginError::Busy);
    }
    let turn_id = uuid::Uuid::now_v7().to_string();
    let session = match &agent.provider_session_id {
        Some(id) => Session::Resume(id.clone()),
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            transaction.execute(
                "UPDATE agents SET provider_session_id = ?2 WHERE id = ?1",
                params![agent_id, id],
            )?;
            Session::New(id)
        }
    };
    transaction.execute(
        "INSERT INTO turns (id, agent_id, status, started_at) VALUES (?1, ?2, 'running', ?3)",
        params![turn_id, agent_id, now],
    )?;
    transaction.execute(
        "INSERT INTO messages (id, agent_id, turn_id, role, content, created_at)
         VALUES (?1, ?2, ?3, 'user', ?4, ?5)",
        params![
            uuid::Uuid::now_v7().to_string(),
            agent_id,
            turn_id,
            text,
            now
        ],
    )?;
    transaction.execute(
        "UPDATE agents SET last_activity_at = ?2 WHERE id = ?1",
        params![agent_id, now],
    )?;
    Ok(Begun {
        turn_id,
        agent,
        session,
    })
}

fn enqueue(
    connection: &Connection,
    agent_id: &str,
    text: &str,
    now: i64,
) -> rusqlite::Result<QueuedMessage> {
    let id = uuid::Uuid::now_v7().to_string();
    let position: i64 = connection.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM queued_messages WHERE agent_id = ?1",
        [agent_id],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO queued_messages (id, agent_id, content, position, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, agent_id, text, position, now],
    )?;
    Ok(QueuedMessage {
        id,
        text: text.to_owned(),
        position,
        created_at: now,
    })
}

pub fn list_queue(connection: &Connection, agent_id: &str) -> rusqlite::Result<Vec<QueuedMessage>> {
    let mut statement = connection.prepare(
        "SELECT id, content, position, created_at FROM queued_messages
         WHERE agent_id = ?1 ORDER BY position",
    )?;
    let rows = statement.query_map([agent_id], |row| {
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
            "UPDATE queued_messages SET content = ?2 WHERE id = ?1
             RETURNING agent_id, id, content, position, created_at",
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
            "DELETE FROM queued_messages WHERE id = ?1 RETURNING agent_id",
            [queued_id],
            |row| row.get(0),
        )
        .optional()
}

fn advance_in(
    transaction: &Connection,
    agent_id: &str,
    now: i64,
) -> Result<Option<Advanced>, BeginError> {
    let running: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns WHERE agent_id = ?1 AND status = 'running')",
        [agent_id],
        |row| row.get(0),
    )?;
    if running {
        return Ok(None);
    }
    let next: Option<(String, String)> = transaction
        .query_row(
            "SELECT id, content FROM queued_messages WHERE agent_id = ?1
             ORDER BY position LIMIT 1",
            [agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((queued_id, text)) = next else {
        return Ok(None);
    };
    transaction.execute("DELETE FROM queued_messages WHERE id = ?1", [queued_id])?;
    let begun = begin_in(transaction, agent_id, &text, now)?;
    Ok(Some(Advanced { begun, text }))
}

pub fn advance_queue(
    connection: &Connection,
    agent_id: &str,
    now: i64,
) -> Result<Option<Advanced>, BeginError> {
    let transaction = connection.unchecked_transaction()?;
    let advanced = advance_in(&transaction, agent_id, now)?;
    transaction.commit()?;
    Ok(advanced)
}

pub fn set_session(
    connection: &Connection,
    agent_id: &str,
    session_id: Option<&str>,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE agents SET provider_session_id = ?2 WHERE id = ?1",
        params![agent_id, session_id],
    )?;
    Ok(())
}

pub struct NewMessage<'a> {
    pub id: &'a str,
    pub agent_id: &'a str,
    pub turn_id: &'a str,
    pub group_id: Option<&'a str>,
    pub role: &'a str,
    pub content: &'a str,
    pub created_at: i64,
}

pub fn insert_message_in(connection: &Connection, message: &NewMessage) -> rusqlite::Result<()> {
    let author = message.group_id.and(Some(message.agent_id));
    connection.execute(
        "INSERT INTO messages (id, agent_id, turn_id, group_id, author_agent_id, role, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id,
            message.agent_id,
            message.turn_id,
            message.group_id,
            author,
            message.role,
            message.content,
            message.created_at
        ],
    )?;
    Ok(())
}

#[cfg(test)]
pub fn insert_message(
    connection: &Connection,
    id: &str,
    agent_id: &str,
    turn_id: &str,
    role: &str,
    content: &str,
    created_at: i64,
) -> rusqlite::Result<()> {
    insert_message_in(
        connection,
        &NewMessage {
            id,
            agent_id,
            turn_id,
            group_id: None,
            role,
            content,
            created_at,
        },
    )
}

pub fn update_message(connection: &Connection, id: &str, content: &str) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE messages SET content = ?2 WHERE id = ?1",
        params![id, content],
    )?;
    Ok(())
}

pub fn insert_step(
    connection: &Connection,
    turn_id: &str,
    position: i64,
    step: &StepView,
    created_at: i64,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO steps (id, turn_id, position, kind, title, detail, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            step.id,
            turn_id,
            position,
            step.kind,
            step.title,
            step.detail,
            step.status,
            created_at
        ],
    )?;
    Ok(())
}

pub fn end_step(
    connection: &Connection,
    id: &str,
    status: &str,
    detail: Option<&str>,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE steps SET status = ?2, detail = ?3 WHERE id = ?1",
        params![id, status, detail],
    )?;
    Ok(())
}

pub fn finish_turn(
    connection: &Connection,
    turn_id: &str,
    agent_id: &str,
    status: &str,
    error: Option<&str>,
    now: i64,
) -> rusqlite::Result<()> {
    let transaction = connection.unchecked_transaction()?;
    finish_turn_rows(&transaction, turn_id, agent_id, status, error, now)?;
    transaction.commit()
}

pub fn finish_turn_rows(
    connection: &Connection,
    turn_id: &str,
    agent_id: &str,
    status: &str,
    error: Option<&str>,
    now: i64,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE steps SET status = 'stopped' WHERE turn_id = ?1 AND status = 'running'",
        [turn_id],
    )?;
    connection.execute(
        "UPDATE turns SET status = ?2, error = ?3, ended_at = ?4 WHERE id = ?1",
        params![turn_id, status, error, now],
    )?;
    connection.execute(
        "UPDATE agents SET last_activity_at = ?2 WHERE id = ?1",
        params![agent_id, now],
    )?;
    Ok(())
}

pub fn sweep_interrupted(connection: &Connection, now: i64) -> rusqlite::Result<usize> {
    let transaction = connection.unchecked_transaction()?;
    let group_turns: Vec<String> = transaction
        .prepare("SELECT id FROM turns WHERE status = 'running' AND group_id IS NOT NULL")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    transaction.execute(
        "UPDATE steps SET status = 'stopped'
         WHERE status = 'running'
           AND turn_id IN (SELECT id FROM turns WHERE status = 'running')",
        [],
    )?;
    let count = transaction.execute(
        "UPDATE turns SET status = 'interrupted', ended_at = ?1 WHERE status = 'running'",
        [now],
    )?;
    super::group::settle_interrupted(&transaction, &group_turns, now)?;
    transaction.commit()?;
    Ok(count)
}

struct TurnRow {
    id: String,
    agent_id: Option<String>,
    status: String,
    error: Option<String>,
    started_at: i64,
    ended_at: Option<i64>,
    kind: String,
    from_agent_id: Option<String>,
}

fn load_items(connection: &Connection, turn_id: &str) -> rusqlite::Result<(String, Vec<ItemView>)> {
    let mut user_message = String::new();
    let mut ordered: Vec<(i64, i64, ItemView)> = Vec::new();
    let mut messages = connection.prepare(
        "SELECT id, role, content, created_at FROM messages WHERE turn_id = ?1 ORDER BY created_at, rowid",
    )?;
    let rows = messages.query_map([turn_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (id, role, content, created_at) = row?;
        match role.as_str() {
            "user" => user_message = content,
            "notice" => ordered.push((created_at, 0, ItemView::Notice { id, text: content })),
            _ => ordered.push((created_at, 0, ItemView::Text { id, text: content })),
        }
    }
    let mut steps = connection.prepare(
        "SELECT id, kind, title, detail, status, created_at FROM steps WHERE turn_id = ?1 ORDER BY position",
    )?;
    let rows = steps.query_map([turn_id], |row| {
        Ok((
            StepView {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                detail: row.get(3)?,
                status: row.get(4)?,
            },
            row.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (step, created_at) = row?;
        ordered.push((created_at, 1, ItemView::Step(step)));
    }
    ordered.sort_by_key(|(created_at, rank, _)| (*created_at, *rank));
    Ok((
        user_message,
        ordered.into_iter().map(|(_, _, item)| item).collect(),
    ))
}

fn load_hand_offs(connection: &Connection, turn_id: &str) -> rusqlite::Result<Vec<HandOffView>> {
    let mut found = Vec::new();
    for sql in [
        "SELECT t.agent_id,
                COALESCE((SELECT m.content FROM messages m
                          WHERE m.turn_id = t.id AND m.role = 'user' LIMIT 1), '')
         FROM turns t
         WHERE t.parent_turn_id = ?1 AND t.kind = 'hand-off' AND t.agent_id IS NOT NULL
         ORDER BY t.rowid",
        "SELECT addressed_agent_id, content FROM group_queue
         WHERE parent_turn_id = ?1 AND kind = 'hand-off'
         ORDER BY position",
    ] {
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map([turn_id], |row| {
            Ok(HandOffView {
                agent_id: row.get(0)?,
                task: row.get(1)?,
            })
        })?;
        for row in rows {
            found.push(row?);
        }
    }
    Ok(found)
}

const AGENT_CHAT: &str = "agent_id = ?1 AND group_id IS NULL";
const GROUP_CHAT: &str = "group_id = ?1";

fn page(
    connection: &Connection,
    filter: &str,
    scope_id: &str,
    before_turn_id: Option<&str>,
) -> rusqlite::Result<ConversationPage> {
    let mut statement = connection.prepare(&format!(
        "SELECT id, agent_id, status, error, started_at, ended_at, kind, from_agent_id FROM turns
         WHERE {filter}
           AND (?2 IS NULL OR rowid < (SELECT rowid FROM turns WHERE id = ?2))
         ORDER BY rowid DESC LIMIT ?3"
    ))?;
    let mut rows: Vec<TurnRow> = statement
        .query_map(
            params![scope_id, before_turn_id, (PAGE_SIZE + 1) as i64],
            |row| {
                Ok(TurnRow {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    status: row.get(2)?,
                    error: row.get(3)?,
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    kind: row.get(6)?,
                    from_agent_id: row.get(7)?,
                })
            },
        )?
        .collect::<rusqlite::Result<_>>()?;
    let has_more = rows.len() > PAGE_SIZE;
    rows.truncate(PAGE_SIZE);
    rows.reverse();
    let turns = rows
        .into_iter()
        .map(|row| {
            let (user_message, items) = load_items(connection, &row.id)?;
            let hand_offs = load_hand_offs(connection, &row.id)?;
            Ok(TurnView {
                id: row.id,
                agent_id: row.agent_id,
                status: row.status,
                error: row.error,
                started_at: row.started_at,
                ended_at: row.ended_at,
                kind: row.kind,
                from_agent_id: row.from_agent_id,
                hand_offs,
                user_message,
                items,
            })
        })
        .collect::<rusqlite::Result<_>>()?;
    Ok(ConversationPage { turns, has_more })
}

pub fn conversation(
    connection: &Connection,
    agent_id: &str,
    before_turn_id: Option<&str>,
) -> rusqlite::Result<ConversationPage> {
    page(connection, AGENT_CHAT, agent_id, before_turn_id)
}

pub fn group_conversation(
    connection: &Connection,
    group_id: &str,
    before_turn_id: Option<&str>,
) -> rusqlite::Result<ConversationPage> {
    page(connection, GROUP_CHAT, group_id, before_turn_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn database() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'default', '/w', 1, 1)",
                [],
            )
            .unwrap();
        });
        (dir, db)
    }

    fn begin(db: &Database, text: &str, now: i64) -> Result<Begun, BeginError> {
        db.with_connection(|c| begin_turn(c, "a", text, now))
    }

    fn finish(db: &Database, turn: &str) {
        db.with_connection(|c| finish_turn(c, turn, "a", "completed", None, 9).unwrap());
    }

    fn count(db: &Database, sql: &str) -> i64 {
        db.with_connection(|c| c.query_row(sql, [], |row| row.get(0)).unwrap())
    }

    #[test]
    fn begin_inserts_the_turn_and_the_user_message_and_touches_the_agent() {
        let (_dir, db) = database();
        let begun = begin(&db, "hello", 50).unwrap();
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM turns WHERE status = 'running'"),
            1
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM messages WHERE role = 'user' AND content = 'hello' AND turn_id IS NOT NULL"),
            1
        );
        assert_eq!(count(&db, "SELECT last_activity_at FROM agents"), 50);
        assert!(matches!(begun.session, Session::New(_)));
    }

    #[test]
    fn the_first_turn_stores_a_new_session_and_later_turns_resume_it() {
        let (_dir, db) = database();
        let first = begin(&db, "one", 10).unwrap();
        let Session::New(id) = first.session else {
            panic!("expected a new session");
        };
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        finish(&db, &first.turn_id);
        let second = begin(&db, "two", 20).unwrap();
        assert_eq!(second.session, Session::Resume(id));
    }

    #[test]
    fn a_second_begin_while_a_turn_runs_is_busy_and_inserts_nothing() {
        let (_dir, db) = database();
        begin(&db, "one", 10).unwrap();
        assert!(matches!(begin(&db, "two", 11), Err(BeginError::Busy)));
        assert_eq!(count(&db, "SELECT COUNT(*) FROM turns"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM messages"), 1);
    }

    #[test]
    fn begin_for_an_unknown_agent_is_not_found() {
        let (_dir, db) = database();
        let result = db.with_connection(|c| begin_turn(c, "missing", "x", 1));
        assert!(matches!(result, Err(BeginError::NotFound)));
    }

    #[test]
    fn a_failed_insert_rolls_the_whole_begin_back() {
        let (_dir, db) = database();
        db.with_connection(|c| {
            c.execute_batch(
                "CREATE TRIGGER fail_user_message BEFORE INSERT ON messages
                 BEGIN SELECT RAISE(ABORT, 'no'); END;",
            )
            .unwrap()
        });
        assert!(matches!(begin(&db, "x", 5), Err(BeginError::Database(_))));
        assert_eq!(count(&db, "SELECT COUNT(*) FROM turns"), 0);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM agents WHERE provider_session_id IS NULL AND last_activity_at = 1"),
            1
        );
    }

    #[test]
    fn the_startup_sweep_turns_running_turns_into_interrupted_and_stops_their_steps() {
        let (_dir, db) = database();
        let running = begin(&db, "one", 10).unwrap();
        db.with_connection(|c| {
            let step = StepView {
                id: "s".into(),
                kind: "Bash".into(),
                title: "ls".into(),
                detail: None,
                status: "running".into(),
            };
            insert_step(c, &running.turn_id, 0, &step, 11).unwrap();
        });
        let swept = db.with_connection(|c| sweep_interrupted(c, 99).unwrap());
        assert_eq!(swept, 1);
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM turns WHERE status = 'interrupted' AND ended_at = 99"
            ),
            1
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM steps WHERE status = 'stopped'"),
            1
        );
        assert!(begin(&db, "two", 100).is_ok());
    }

    #[test]
    fn the_conversation_pages_by_twenty_turns_oldest_first() {
        let (_dir, db) = database();
        for n in 0..45 {
            let begun = begin(&db, &format!("m{n}"), 100 + n).unwrap();
            finish(&db, &begun.turn_id);
        }
        let newest = db.with_connection(|c| conversation(c, "a", None).unwrap());
        assert_eq!(newest.turns.len(), 20);
        assert!(newest.has_more);
        assert_eq!(newest.turns[0].user_message, "m25");
        assert_eq!(newest.turns[19].user_message, "m44");
        let middle =
            db.with_connection(|c| conversation(c, "a", Some(&newest.turns[0].id)).unwrap());
        assert_eq!(middle.turns.len(), 20);
        assert!(middle.has_more);
        assert_eq!(middle.turns[0].user_message, "m5");
        assert_eq!(middle.turns[19].user_message, "m24");
        let oldest =
            db.with_connection(|c| conversation(c, "a", Some(&middle.turns[0].id)).unwrap());
        assert_eq!(oldest.turns.len(), 5);
        assert!(!oldest.has_more);
        assert_eq!(oldest.turns[0].user_message, "m0");
    }

    #[test]
    fn items_come_back_in_the_order_they_were_written() {
        let (_dir, db) = database();
        let begun = begin(&db, "go", 10).unwrap();
        db.with_connection(|c| {
            insert_message(c, "t1", "a", &begun.turn_id, "assistant", "first", 11).unwrap();
            let step = StepView {
                id: "s1".into(),
                kind: "Bash".into(),
                title: "ls".into(),
                detail: Some("out".into()),
                status: "done".into(),
            };
            insert_step(c, &begun.turn_id, 0, &step, 12).unwrap();
            insert_message(c, "n1", "a", &begun.turn_id, "notice", "heads up", 13).unwrap();
            insert_message(c, "t2", "a", &begun.turn_id, "assistant", "second", 14).unwrap();
        });
        let page = db.with_connection(|c| conversation(c, "a", None).unwrap());
        let items = &page.turns[0].items;
        assert_eq!(items.len(), 4);
        assert!(matches!(&items[0], ItemView::Text { text, .. } if text == "first"));
        assert!(matches!(&items[1], ItemView::Step(step) if step.title == "ls"));
        assert!(matches!(&items[2], ItemView::Notice { .. }));
        assert!(matches!(&items[3], ItemView::Text { text, .. } if text == "second"));
        assert_eq!(page.turns[0].user_message, "go");
    }
}
