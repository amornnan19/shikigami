use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::claude_code::Session;
use super::store::{self, BeginError, Begun, QueuedMessage};
use crate::groups::{self, mentions, Member};
use crate::providers::CLAUDE_CODE_ID;

mod hand_offs;

pub use hand_offs::{drop_for_departed, settle_interrupted, Notice};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccessLevel(u8);

impl AccessLevel {
    pub const STANDARD: Self = Self(1);

    pub fn permission_mode(self) -> &'static str {
        #[cfg(test)]
        if self < Self::STANDARD {
            return "manual-lowest";
        }
        "manual"
    }

    fn value(self) -> i64 {
        i64::from(self.0)
    }

    fn from_value(value: i64) -> Self {
        Self(u8::try_from(value).unwrap_or(0))
    }
}

#[cfg(test)]
thread_local! {
    static TEST_LEVELS: std::cell::RefCell<std::collections::HashMap<String, AccessLevel>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

#[cfg(test)]
fn set_test_level(agent_id: &str, level: AccessLevel) {
    TEST_LEVELS.with(|levels| levels.borrow_mut().insert(agent_id.to_owned(), level));
}

fn access_level_of(agent_id: &str) -> AccessLevel {
    #[cfg(test)]
    if let Some(level) = TEST_LEVELS.with(|levels| levels.borrow().get(agent_id).copied()) {
        return level;
    }
    let _ = agent_id;
    AccessLevel::STANDARD
}

fn lower_access(first: AccessLevel, second: AccessLevel) -> AccessLevel {
    first.min(second)
}

pub fn access_for_hand_off(causing: AccessLevel, to_agent_id: &str) -> AccessLevel {
    lower_access(causing, access_level_of(to_agent_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    User,
    HandOff,
    Continuation,
}

impl EntryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::HandOff => "hand-off",
            Self::Continuation => "continuation",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "hand-off" => Self::HandOff,
            "continuation" => Self::Continuation,
            _ => Self::User,
        }
    }
}

pub const MAX_ADDRESSED: usize = 8;
pub const TRUNCATED_NOTICE: &str = "Only the first 8 mentioned agents were addressed.";

#[derive(Debug, Clone)]
pub struct Origin {
    pub kind: EntryKind,
    pub from_agent_id: Option<String>,
    pub root: Option<String>,
    pub hop: i64,
    pub parent_turn_id: Option<String>,
    pub owes_turn_id: Option<String>,
    pub access: AccessLevel,
    pub notice: Option<String>,
}

impl Origin {
    fn user(root: &str, addressed: &str, truncated: bool) -> Self {
        Self {
            kind: EntryKind::User,
            from_agent_id: None,
            root: Some(root.to_owned()),
            hop: 0,
            parent_turn_id: None,
            owes_turn_id: None,
            access: access_level_of(addressed),
            notice: truncated.then(|| TRUNCATED_NOTICE.to_owned()),
        }
    }
}

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
    pub access: AccessLevel,
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

fn resolve_addressed(members: &[Member], lead: String, text: &str) -> (Vec<String>, bool) {
    let mut addressed = mentions::addressed(text, members);
    if addressed.is_empty() {
        addressed.push(lead);
    }
    let truncated = addressed.len() > MAX_ADDRESSED;
    addressed.truncate(MAX_ADDRESSED);
    (addressed, truncated)
}

pub fn addressed_agents(
    connection: &Connection,
    group_id: &str,
    text: &str,
) -> Result<Vec<String>, BeginError> {
    let members = groups::store::members_of(connection, group_id)?;
    let lead = lead_of(connection, group_id)?;
    Ok(resolve_addressed(&members, lead, text).0)
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
    exclude_same: Option<(&str, &str)>,
) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT m.role, m.content, a.name, t.kind FROM messages m
         LEFT JOIN agents a ON a.id = m.author_agent_id
         LEFT JOIN turns t ON t.id = m.turn_id
         WHERE m.group_id = ?1 AND m.seq > ?3 AND m.role IN ('user', 'assistant')
           AND m.author_agent_id IS NOT ?2
           AND m.content <> ''
           AND NOT (m.role = 'user' AND t.kind = 'continuation')
           AND NOT (?4 IS NOT NULL AND m.role = 'user' AND t.kind = 'user'
                    AND COALESCE(t.root_message_id = ?4, 0) AND m.content = ?5)
         ORDER BY m.seq",
    )?;
    let rows = statement.query_map(
        params![
            group_id,
            agent_id,
            after_rowid,
            exclude_same.map(|(root, _)| root),
            exclude_same.map(|(_, text)| text)
        ],
        |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            let name: Option<String> = row.get(2)?;
            let kind: Option<String> = row.get(3)?;
            let from_user = role == "user" && kind.as_deref().is_none_or(|kind| kind == "user");
            let author = match name {
                _ if from_user => "User".to_owned(),
                Some(name) => name,
                None => "Former member".to_owned(),
            };
            Ok(format!("{author}: {content}"))
        },
    )?;
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
    origin: &Origin,
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
    let user_root = origin
        .root
        .as_deref()
        .filter(|_| origin.kind == EntryKind::User);
    let stored_text: Option<String> = match user_root {
        Some(root) => transaction
            .query_row(
                "SELECT m.content FROM messages m JOIN turns t ON t.id = m.turn_id
                 WHERE t.group_id = ?1 AND t.root_message_id = ?2 AND t.kind = 'user'
                   AND m.role = 'user'
                 ORDER BY m.seq LIMIT 1",
                params![group_id, root],
                |row| row.get(0),
            )
            .optional()?,
        None => None,
    };
    let same_text = stored_text.as_deref() == Some(text);
    let exclude = user_root.filter(|_| same_text);
    let unseen = unseen_lines(
        transaction,
        group_id,
        addressed,
        last_seen,
        exclude.map(|root| (root, text)),
    )?;
    let access = access_for_hand_off(origin.access, addressed);
    let turn_id = uuid::Uuid::now_v7().to_string();
    transaction.execute(
        "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id, hop, kind,
                            from_agent_id, parent_turn_id, owes_turn_id, access)
         VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            turn_id,
            addressed,
            group_id,
            now,
            origin.root,
            origin.hop,
            origin.kind.as_str(),
            origin.from_agent_id,
            origin.parent_turn_id,
            origin.owes_turn_id,
            access.value()
        ],
    )?;
    let message_stored = same_text;
    if !message_stored {
        let author = (origin.kind == EntryKind::HandOff)
            .then_some(origin.from_agent_id.as_deref())
            .flatten();
        transaction.execute(
            "INSERT INTO messages (id, agent_id, turn_id, group_id, author_agent_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'user', ?6, ?7)",
            params![
                uuid::Uuid::now_v7().to_string(),
                addressed,
                turn_id,
                group_id,
                author,
                text,
                now
            ],
        )?;
    }
    if let Some(notice) = &origin.notice {
        transaction.execute(
            "INSERT INTO messages (id, agent_id, turn_id, group_id, author_agent_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?2, 'notice', ?5, ?6)",
            params![
                uuid::Uuid::now_v7().to_string(),
                addressed,
                turn_id,
                group_id,
                notice,
                now
            ],
        )?;
    }
    let message_seq: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM messages WHERE group_id = ?1",
        [group_id],
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
        access,
    })
}

fn enqueue(
    connection: &Connection,
    group_id: &str,
    addressed: &str,
    text: &str,
    origin: &Origin,
    now: i64,
) -> rusqlite::Result<QueuedMessage> {
    let position: i64 = connection.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM group_queue WHERE group_id = ?1",
        [group_id],
        |row| row.get(0),
    )?;
    enqueue_at(connection, group_id, addressed, text, origin, position, now)
}

fn enqueue_at(
    connection: &Connection,
    group_id: &str,
    addressed: &str,
    text: &str,
    origin: &Origin,
    position: i64,
    now: i64,
) -> rusqlite::Result<QueuedMessage> {
    let id = uuid::Uuid::now_v7().to_string();
    connection.execute(
        "INSERT INTO group_queue (id, group_id, addressed_agent_id, content, from_agent_id, root_message_id, hop, kind,
                                 parent_turn_id, owes_turn_id, access, notice, position, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            id,
            group_id,
            addressed,
            text,
            origin.from_agent_id,
            origin.root,
            origin.hop,
            origin.kind.as_str(),
            origin.parent_turn_id,
            origin.owes_turn_id,
            origin.access.value(),
            origin.notice,
            position,
            now
        ],
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
    let facts = facts(&transaction, group_id)?;
    let lead = lead_of(&transaction, group_id)?;
    let (addressed, truncated) = resolve_addressed(&facts.members, lead, text);
    let root = uuid::Uuid::now_v7().to_string();
    let backlog: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_queue WHERE group_id = ?1)",
        [group_id],
        |row| row.get(0),
    )?;
    let mut entries = addressed.iter().enumerate();
    let mut started = None;
    let mut first_queued = None;
    if !backlog {
        if let Some((_, first)) = entries.next() {
            let origin = Origin::user(&root, first, truncated);
            match begin_in(&transaction, group_id, first, text, &origin, now) {
                Ok(begun) => started = Some(begun),
                Err(BeginError::Busy) => {
                    first_queued =
                        Some(enqueue(&transaction, group_id, first, text, &origin, now)?);
                }
                Err(error) => return Err(error),
            }
        }
    }
    for (index, entry) in entries {
        let origin = Origin::user(&root, entry, truncated && backlog && index == 0);
        let queued = enqueue(&transaction, group_id, entry, text, &origin, now)?;
        first_queued.get_or_insert(queued);
    }
    if backlog {
        started = advance_in(&transaction, group_id, now)?;
    }
    let sent = match (started, first_queued) {
        (Some(begun), _) if !backlog => GroupSent::Started(begun),
        (started, Some(queued)) => GroupSent::Queued { queued, started },
        (_, None) => return Err(BeginError::Busy),
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
    let head: Option<(String, String, String, Origin)> = transaction
        .query_row(
            "SELECT id, addressed_agent_id, content, kind, from_agent_id, root_message_id, hop,
                    parent_turn_id, owes_turn_id, access, notice
             FROM group_queue WHERE group_id = ?1
             ORDER BY position LIMIT 1",
            [group_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    Origin {
                        kind: EntryKind::parse(&row.get::<_, String>(3)?),
                        from_agent_id: row.get(4)?,
                        root: row.get(5)?,
                        hop: row.get(6)?,
                        parent_turn_id: row.get(7)?,
                        owes_turn_id: row.get(8)?,
                        access: AccessLevel::from_value(row.get(9)?),
                        notice: row.get(10)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((queued_id, mut addressed, text, origin)) = head else {
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
    begin_in(transaction, group_id, &addressed, &text, &origin, now).map(Some)
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

pub fn finish_turn(connection: &Connection, finish: &Finish) -> rusqlite::Result<Vec<Notice>> {
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
    let notices = match status {
        "completed" => hand_offs::queue_after(&transaction, turn_id, now)?,
        "stopped" => hand_offs::drop_after_stop(&transaction, turn_id, now)?,
        "failed" => hand_offs::unfinished_after(&transaction, turn_id, now)?,
        _ => Vec::new(),
    };
    transaction.commit()?;
    Ok(notices)
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
