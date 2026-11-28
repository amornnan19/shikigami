use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalView {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub detail: String,
    pub reason: Option<String>,
    pub target: Option<String>,
    pub status: String,
    pub answered_by: Option<String>,
    pub created_at: i64,
    pub answered_at: Option<i64>,
    pub folder: String,
}

pub struct NewApproval<'a> {
    pub id: &'a str,
    pub turn_id: &'a str,
    pub tool: &'a str,
    pub summary: &'a str,
    pub detail: &'a str,
    pub reason: Option<&'a str>,
    pub target: Option<&'a str>,
    pub created_at: i64,
}

pub struct Outcome {
    pub status: &'static str,
    pub answered_by: &'static str,
}

pub const ALLOWED_BY_USER: Outcome = Outcome {
    status: "allowed",
    answered_by: "user",
};
pub const DENIED_BY_USER: Outcome = Outcome {
    status: "denied",
    answered_by: "user",
};
pub const TIMED_OUT: Outcome = Outcome {
    status: "timed-out",
    answered_by: "timeout",
};
pub const DENIED_BY_STOP: Outcome = Outcome {
    status: "denied",
    answered_by: "stop",
};
pub const DENIED_BY_RELAY: Outcome = Outcome {
    status: "denied",
    answered_by: "relay",
};
pub const DENIED_BY_SHUTDOWN: Outcome = Outcome {
    status: "denied",
    answered_by: "shutdown",
};

const VIEW_COLUMNS: &str =
    "a.id, a.tool, a.summary, a.detail, a.reason, a.target, a.status, a.answered_by,
        a.created_at, a.answered_at, COALESCE(g.folder_path, ag.workspace_path, '')";
const VIEW_JOINS: &str = "FROM approvals a
        JOIN turns t ON t.id = a.turn_id
        LEFT JOIN groups g ON g.id = t.group_id
        LEFT JOIN agents ag ON ag.id = t.agent_id";

fn view_of(row: &rusqlite::Row) -> rusqlite::Result<ApprovalView> {
    Ok(ApprovalView {
        id: row.get(0)?,
        tool: row.get(1)?,
        summary: row.get(2)?,
        detail: row.get(3)?,
        reason: row.get(4)?,
        target: row.get(5)?,
        status: row.get(6)?,
        answered_by: row.get(7)?,
        created_at: row.get(8)?,
        answered_at: row.get(9)?,
        folder: row.get(10)?,
    })
}

pub fn insert(connection: &Connection, approval: &NewApproval) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO approvals (id, turn_id, tool, summary, detail, reason, target, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)",
        params![
            approval.id,
            approval.turn_id,
            approval.tool,
            approval.summary,
            approval.detail,
            approval.reason,
            approval.target,
            approval.created_at
        ],
    )?;
    Ok(())
}

pub fn resolve(
    connection: &Connection,
    id: &str,
    outcome: &Outcome,
    now: i64,
) -> rusqlite::Result<bool> {
    let changed = connection.execute(
        "UPDATE approvals SET status = ?2, answered_by = ?3, answered_at = ?4
         WHERE id = ?1 AND status = 'pending'",
        params![id, outcome.status, outcome.answered_by, now],
    )?;
    Ok(changed == 1)
}

pub fn view(connection: &Connection, id: &str) -> rusqlite::Result<Option<ApprovalView>> {
    connection
        .query_row(
            &format!("SELECT {VIEW_COLUMNS} {VIEW_JOINS} WHERE a.id = ?1"),
            [id],
            view_of,
        )
        .optional()
}

pub fn for_turn(connection: &Connection, turn_id: &str) -> rusqlite::Result<Vec<ApprovalView>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {VIEW_COLUMNS} {VIEW_JOINS} WHERE a.turn_id = ?1 ORDER BY a.created_at, a.rowid"
    ))?;
    let rows = statement.query_map([turn_id], view_of)?;
    rows.collect()
}

pub struct Target {
    pub turn_id: String,
    pub status: String,
}

pub fn target(connection: &Connection, id: &str) -> rusqlite::Result<Option<Target>> {
    connection
        .query_row(
            "SELECT turn_id, status FROM approvals WHERE id = ?1",
            [id],
            |row| {
                Ok(Target {
                    turn_id: row.get(0)?,
                    status: row.get(1)?,
                })
            },
        )
        .optional()
}

pub fn sweep_pending(connection: &Connection, now: i64) -> rusqlite::Result<usize> {
    connection.execute(
        "UPDATE approvals SET status = 'denied', answered_by = 'shutdown', answered_at = ?1
         WHERE status = 'pending'",
        [now],
    )
}

pub fn group_waiting(connection: &Connection, group_id: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM approvals a JOIN turns t ON t.id = a.turn_id
                        WHERE t.group_id = ?1 AND a.status = 'pending')",
        [group_id],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::turns;

    fn database() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/agents/ada', 1, 1);
                 INSERT INTO groups VALUES ('g', 'Team', '/projects/site', 'a', 1, 1);
                 INSERT INTO turns (id, agent_id, status, started_at) VALUES ('t1', 'a', 'running', 1);
                 INSERT INTO turns (id, agent_id, status, started_at, group_id) VALUES ('t2', 'a', 'running', 2, 'g');",
            )
            .unwrap();
        });
        (dir, db)
    }

    fn add(db: &Database, id: &str, turn: &str, created_at: i64) {
        db.with_connection(|c| {
            insert(
                c,
                &NewApproval {
                    id,
                    turn_id: turn,
                    tool: "Bash",
                    summary: "ls",
                    detail: "{}",
                    reason: Some("why"),
                    target: None,
                    created_at,
                },
            )
            .unwrap()
        });
    }

    #[test]
    fn a_card_shows_the_agent_folder_in_a_chat_and_the_group_folder_in_a_group() {
        let (_dir, db) = database();
        add(&db, "own", "t1", 1);
        add(&db, "shared", "t2", 2);
        let folder = |id: &str| db.with_connection(|c| view(c, id).unwrap().unwrap()).folder;
        assert_eq!(folder("own"), "/agents/ada");
        assert_eq!(folder("shared"), "/projects/site");
    }

    #[test]
    fn only_a_pending_row_can_be_resolved_and_only_once() {
        let (_dir, db) = database();
        add(&db, "x", "t1", 1);
        assert!(db.with_connection(|c| resolve(c, "x", &ALLOWED_BY_USER, 5).unwrap()));
        assert!(!db.with_connection(|c| resolve(c, "x", &DENIED_BY_USER, 6).unwrap()));
        let row = db.with_connection(|c| view(c, "x").unwrap().unwrap());
        assert_eq!(row.status, "allowed");
        assert_eq!(row.answered_by.as_deref(), Some("user"));
        assert_eq!(row.answered_at, Some(5));
    }

    #[test]
    fn a_group_with_an_open_card_waits_for_the_user_and_an_answered_one_does_not() {
        let (_dir, db) = database();
        assert!(!db.with_connection(|c| group_waiting(c, "g").unwrap()));
        add(&db, "x", "t2", 1);
        assert!(db.with_connection(|c| group_waiting(c, "g").unwrap()));
        db.with_connection(|c| resolve(c, "x", &TIMED_OUT, 2).unwrap());
        assert!(!db.with_connection(|c| group_waiting(c, "g").unwrap()));
    }

    #[test]
    fn the_startup_sweep_denies_open_cards_as_shutdown_and_keeps_answered_ones() {
        let (_dir, db) = database();
        add(&db, "open", "t1", 1);
        add(&db, "done", "t1", 2);
        db.with_connection(|c| resolve(c, "done", &ALLOWED_BY_USER, 3).unwrap());
        turns::sweep_interrupted(&db).unwrap();
        let open = db.with_connection(|c| view(c, "open").unwrap().unwrap());
        assert_eq!(open.status, "denied");
        assert_eq!(open.answered_by.as_deref(), Some("shutdown"));
        assert!(open.answered_at.is_some());
        let done = db.with_connection(|c| view(c, "done").unwrap().unwrap());
        assert_eq!(done.status, "allowed");
        assert_eq!(done.answered_by.as_deref(), Some("user"));
    }

    #[test]
    fn past_cards_load_with_their_turn_in_order_after_a_restart() {
        let (_dir, db) = database();
        add(&db, "b", "t1", 20);
        add(&db, "a", "t1", 10);
        db.with_connection(|c| resolve(c, "a", &DENIED_BY_USER, 11).unwrap());
        let loaded = db.with_connection(|c| for_turn(c, "t1").unwrap());
        let ids: Vec<&str> = loaded.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }
}
