mod error;

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use refinery::Runner;
use rusqlite::Connection;

pub use error::DbError;

mod embedded {
    refinery::embed_migrations!("migrations");
}

pub const DATABASE_FILE: &str = "shikigami.db";

pub struct Database {
    connection: Mutex<Connection>,
}

fn configure(connection: &Connection) -> Result<(), DbError> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = OFF;",
        )
        .map_err(DbError::Pragma)
}

fn migrate(connection: &mut Connection, runner: Runner) -> Result<(), DbError> {
    runner.set_grouped(true).run(connection)?;
    let violations = foreign_key_violations(connection).map_err(DbError::Pragma)?;
    if violations > 0 {
        return Err(DbError::ForeignKeyViolations(violations));
    }
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(DbError::Pragma)
}

fn foreign_key_violations(connection: &Connection) -> rusqlite::Result<usize> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while rows.next()?.is_some() {
        count += 1;
    }
    Ok(count)
}

impl Database {
    pub fn open(data_dir: &Path) -> Result<Self, DbError> {
        fs::create_dir_all(data_dir).map_err(|source| DbError::CreateDir {
            path: data_dir.to_path_buf(),
            source,
        })?;
        let path = data_dir.join(DATABASE_FILE);
        let mut connection = Connection::open(&path).map_err(|source| DbError::Open {
            path: path.clone(),
            source,
        })?;
        configure(&connection)?;
        migrate(&mut connection, embedded::migrations::runner())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn with_connection<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        let guard = self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_version(db: &Database) -> u32 {
        db.with_connection(|c| {
            c.query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get(0),
            )
            .unwrap()
        })
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.with_connection(|c| {
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
    }

    fn seed_agent_tree(db: &Database) {
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'codex', 'm', '/w', 1, 1);
                 INSERT INTO turns (id, agent_id, status, error, started_at, ended_at)
                 VALUES ('t', 'a', 'running', NULL, 1, NULL);
                 INSERT INTO messages (id, agent_id, turn_id, role, content, created_at)
                 VALUES ('m', 'a', 't', 'user', 'hi', 1);
                 INSERT INTO steps VALUES ('s', 't', 0, 'tool', 'x', NULL, 'running', 1);
                 INSERT INTO queued_messages VALUES ('q', 'a', 'later', 0, 1);
                 INSERT INTO groups VALUES ('g', 'Team', '/g', 'a', 1, 1);
                 INSERT INTO group_members VALUES ('g', 'a', 0, 1);",
            )
            .unwrap()
        })
    }

    #[test]
    fn fresh_database_migrates_to_the_latest_version_with_all_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 7);
        for table in [
            "agents",
            "turns",
            "messages",
            "steps",
            "queued_messages",
            "settings",
            "groups",
            "group_members",
            "group_queue",
            "agent_group_sessions",
        ] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
    }

    #[test]
    fn the_settings_migration_keeps_the_agents_of_a_version_one_database() {
        let dir = tempfile::tempdir().unwrap();
        let mut connection = Connection::open(dir.path().join(DATABASE_FILE)).unwrap();
        configure(&connection).unwrap();
        let v1 = [refinery::Migration::unapplied(
            "V1__initial",
            include_str!("../../migrations/V1__initial.sql"),
        )
        .unwrap()];
        migrate(&mut connection, Runner::new(&v1)).unwrap();
        drop(connection);
        let connection = Connection::open(dir.path().join(DATABASE_FILE)).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents VALUES ('a', 'Ada', 'red', 'codex', 'm', '/w', NULL, 1, 1);
                 INSERT INTO turns VALUES ('t', 'a', 'running', NULL, 1, NULL);",
            )
            .unwrap();
        drop(connection);
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 7);
        assert_eq!(count(&db, "agents"), 1);
        assert_eq!(count(&db, "turns"), 1);
        assert_eq!(count(&db, "settings"), 0);
    }

    #[test]
    fn opening_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        drop(Database::open(dir.path()).unwrap());
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 7);
        assert_eq!(count(&db, "refinery_schema_history"), 7);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let result = db.with_connection(|c| {
            c.execute(
                "INSERT INTO turns (id, agent_id, status, error, started_at, ended_at)
                 VALUES ('t', 'missing', 'running', NULL, 1, NULL)",
                [],
            )
        });
        let error = result.unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::ConstraintViolation)
        );
        assert!(error.to_string().contains("FOREIGN KEY"), "{error}");
    }

    #[test]
    fn journal_mode_is_wal() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let mode: String = db.with_connection(|c| {
            c.query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap()
        });
        assert_eq!(mode, "wal");
    }

    #[test]
    fn deleting_an_agent_cascades() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        seed_agent_tree(&db);
        db.with_connection(|c| {
            c.execute("UPDATE turns SET status = 'completed'", [])
                .unwrap()
        });
        crate::agents::remove::delete(&db, &Err(String::new()), "a", false, &|_| Ok(())).unwrap();
        for table in [
            "turns",
            "messages",
            "steps",
            "queued_messages",
            "group_members",
        ] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
        assert_eq!(count(&db, "groups"), 1);
    }

    #[test]
    fn a_file_that_is_not_a_database_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(DATABASE_FILE),
            "this is not a database file".repeat(50),
        )
        .unwrap();
        assert!(Database::open(dir.path()).is_err());
    }

    #[test]
    fn a_failing_migration_leaves_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        drop(Database::open(dir.path()).unwrap());
        let mut connection = Connection::open(dir.path().join(DATABASE_FILE)).unwrap();
        let migrations = [
            refinery::Migration::unapplied(
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V2__settings",
                include_str!("../../migrations/V2__settings.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V3__auth_mode",
                include_str!("../../migrations/V3__auth_mode.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V4__groups",
                include_str!("../../migrations/V4__groups.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V5__group_turns",
                include_str!("../../migrations/V5__group_turns.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V6__group_history",
                include_str!("../../migrations/V6__group_history.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V7__group_context",
                include_str!("../../migrations/V7__group_context.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V8__broken",
                "CREATE TABLE probe (x INTEGER); INSERT INTO missing_table VALUES (1);",
            )
            .unwrap(),
        ];
        let result = migrate(&mut connection, Runner::new(&migrations));
        assert!(result.is_err());
        let version: u32 = connection
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 7);
        let probe_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(probe_tables, 0);
    }

    #[test]
    fn a_failing_migration_rolls_back_every_pending_migration() {
        let dir = tempfile::tempdir().unwrap();
        let mut connection = Connection::open(dir.path().join(DATABASE_FILE)).unwrap();
        let migrations = [
            refinery::Migration::unapplied(
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied("V2__broken", "INSERT INTO missing_table VALUES (1);")
                .unwrap(),
        ];
        assert!(migrate(&mut connection, Runner::new(&migrations)).is_err());
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'agents'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn rebuilding_a_parent_table_keeps_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        seed_agent_tree(&db);
        drop(db);
        let mut connection = Connection::open(dir.path().join(DATABASE_FILE)).unwrap();
        configure(&connection).unwrap();
        let migrations = [
            refinery::Migration::unapplied(
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V2__settings",
                include_str!("../../migrations/V2__settings.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V3__auth_mode",
                include_str!("../../migrations/V3__auth_mode.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V4__groups",
                include_str!("../../migrations/V4__groups.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V5__group_turns",
                include_str!("../../migrations/V5__group_turns.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V6__group_history",
                include_str!("../../migrations/V6__group_history.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V7__group_context",
                include_str!("../../migrations/V7__group_context.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V8__rebuild_agents",
                "CREATE TABLE agents_new (
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                   color TEXT NOT NULL,
                   provider TEXT NOT NULL CHECK (provider IN ('claude-code', 'codex')),
                   model TEXT NOT NULL,
                   workspace_path TEXT NOT NULL,
                   provider_session_id TEXT,
                   created_at INTEGER NOT NULL,
                   last_activity_at INTEGER NOT NULL,
                   auth_mode TEXT
                 );
                 INSERT INTO agents_new SELECT * FROM agents;
                 DROP TABLE agents;
                 ALTER TABLE agents_new RENAME TO agents;",
            )
            .unwrap(),
        ];
        migrate(&mut connection, Runner::new(&migrations)).unwrap();
        let db = Database {
            connection: Mutex::new(connection),
        };
        assert_eq!(schema_version(&db), 8);
        for table in [
            "agents",
            "turns",
            "messages",
            "steps",
            "queued_messages",
            "groups",
            "group_members",
        ] {
            assert_eq!(count(&db, table), 1, "{table}");
        }
        let foreign_keys: i64 = db.with_connection(|c| {
            c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                .unwrap()
        });
        assert_eq!(foreign_keys, 1);
    }

    fn version_two_database(dir: &Path, old_switch: Option<&str>) {
        let mut connection = Connection::open(dir.join(DATABASE_FILE)).unwrap();
        configure(&connection).unwrap();
        let v2 = [
            refinery::Migration::unapplied(
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V2__settings",
                include_str!("../../migrations/V2__settings.sql"),
            )
            .unwrap(),
        ];
        migrate(&mut connection, Runner::new(&v2)).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/w', 's', 1, 1);
                 INSERT INTO turns VALUES ('t', 'a', 'completed', NULL, 1, 2);
                 INSERT INTO settings VALUES ('other', 'kept');",
            )
            .unwrap();
        if let Some(value) = old_switch {
            connection
                .execute(
                    "INSERT INTO settings VALUES ('claude-code.use-api-key', ?1)",
                    [value],
                )
                .unwrap();
        }
    }

    fn setting(db: &Database, key: &str) -> Option<String> {
        db.with_connection(|c| crate::settings::get(c, key).unwrap())
    }

    #[test]
    fn the_auth_mode_migration_converts_the_old_switch_and_keeps_the_agents() {
        for (old, expected) in [
            (Some("true"), Some("api-key")),
            (Some("false"), Some("subscription")),
            (Some("maybe"), None),
            (None, None),
        ] {
            let dir = tempfile::tempdir().unwrap();
            version_two_database(dir.path(), old);
            let db = Database::open(dir.path()).unwrap();
            assert_eq!(schema_version(&db), 7);
            assert_eq!(
                setting(&db, "claude.auth-mode").as_deref(),
                expected,
                "{old:?}"
            );
            assert_eq!(setting(&db, "claude-code.use-api-key"), None, "{old:?}");
            assert_eq!(setting(&db, "other").as_deref(), Some("kept"));
            assert_eq!(count(&db, "agents"), 1);
            assert_eq!(count(&db, "turns"), 1);
            let (session, mode): (String, Option<String>) = db.with_connection(|c| {
                c.query_row(
                    "SELECT provider_session_id, auth_mode FROM agents",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
            });
            assert_eq!((session.as_str(), mode), ("s", None));
        }
    }

    fn version_three_database(dir: &Path) {
        let mut connection = Connection::open(dir.join(DATABASE_FILE)).unwrap();
        configure(&connection).unwrap();
        let v3 = [
            refinery::Migration::unapplied(
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V2__settings",
                include_str!("../../migrations/V2__settings.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V3__auth_mode",
                include_str!("../../migrations/V3__auth_mode.sql"),
            )
            .unwrap(),
        ];
        migrate(&mut connection, Runner::new(&v3)).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at, auth_mode)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/w', 1, 1, 'api-key');
                 INSERT INTO turns VALUES ('t', 'a', 'completed', NULL, 1, 2);
                 INSERT INTO messages VALUES ('m', 'a', 't', 'user', 'hi', 1);
                 INSERT INTO queued_messages VALUES ('q', 'a', 'later', 0, 1);
                 INSERT INTO settings VALUES ('other', 'kept');",
            )
            .unwrap();
    }

    #[test]
    fn the_groups_migration_keeps_everything_of_a_version_three_database() {
        let dir = tempfile::tempdir().unwrap();
        version_three_database(dir.path());
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 7);
        for table in ["agents", "turns", "messages", "queued_messages", "settings"] {
            assert_eq!(count(&db, table), 1, "{table}");
        }
        for table in ["groups", "group_members"] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
        let mode: String = db.with_connection(|c| {
            c.query_row("SELECT auth_mode FROM agents", [], |row| row.get(0))
                .unwrap()
        });
        assert_eq!(mode, "api-key");
        assert_eq!(setting(&db, "other").as_deref(), Some("kept"));
    }

    fn version_four_database(dir: &Path) {
        let mut connection = Connection::open(dir.join(DATABASE_FILE)).unwrap();
        configure(&connection).unwrap();
        let v4 = [
            refinery::Migration::unapplied(
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V2__settings",
                include_str!("../../migrations/V2__settings.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V3__auth_mode",
                include_str!("../../migrations/V3__auth_mode.sql"),
            )
            .unwrap(),
            refinery::Migration::unapplied(
                "V4__groups",
                include_str!("../../migrations/V4__groups.sql"),
            )
            .unwrap(),
        ];
        migrate(&mut connection, Runner::new(&v4)).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, provider_session_id, created_at, last_activity_at, auth_mode)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/a', 'session-a', 1, 1, 'api-key'),
                        ('b', 'Bo', 'blue', 'claude-code', 'm', '/b', NULL, 1, 1, NULL);
                 INSERT INTO turns VALUES ('t', 'a', 'completed', NULL, 1, 2);
                 INSERT INTO messages VALUES ('m', 'a', 't', 'user', 'hi', 1);
                 INSERT INTO steps VALUES ('s', 't', 0, 'tool', 'x', NULL, 'done', 1);
                 INSERT INTO queued_messages VALUES ('q', 'a', 'later', 0, 1);
                 INSERT INTO groups VALUES ('g', 'Team', '/g', 'a', 1, 1);
                 INSERT INTO group_members VALUES ('g', 'a', 0, 1), ('g', 'b', 1, 1);
                 INSERT INTO settings VALUES ('other', 'kept');",
            )
            .unwrap();
    }

    #[test]
    fn the_group_turns_migration_keeps_everything_of_a_version_four_database() {
        let dir = tempfile::tempdir().unwrap();
        version_four_database(dir.path());
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 7);
        for table in [
            "turns",
            "messages",
            "steps",
            "queued_messages",
            "groups",
            "settings",
        ] {
            assert_eq!(count(&db, table), 1, "{table}");
        }
        assert_eq!(count(&db, "agents"), 2);
        assert_eq!(count(&db, "group_members"), 2);
        for table in ["group_queue", "agent_group_sessions"] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
        let (turn_group, message_group, author): (Option<String>, Option<String>, Option<String>) =
            db.with_connection(|c| {
                c.query_row(
                    "SELECT t.group_id, m.group_id, m.author_agent_id
                     FROM turns t JOIN messages m ON m.turn_id = t.id",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
            });
        assert_eq!((turn_group, message_group, author), (None, None, None));
        let (session, mode): (String, String) = db.with_connection(|c| {
            c.query_row(
                "SELECT provider_session_id, auth_mode FROM agents WHERE id = 'a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        });
        assert_eq!((session.as_str(), mode.as_str()), ("session-a", "api-key"));
        assert_eq!(setting(&db, "other").as_deref(), Some("kept"));
    }

    fn seed_group_chat(db: &Database) {
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/a', 1, 1),
                        ('b', 'Bo', 'red', 'claude-code', 'm', '/b', 1, 1);
                 INSERT INTO groups VALUES ('g', 'Team', '/g', 'a', 1, 1);
                 INSERT INTO group_members VALUES ('g', 'a', 0, 1), ('g', 'b', 1, 1);
                 INSERT INTO turns (id, agent_id, group_id, status, started_at)
                 VALUES ('t', 'a', 'g', 'completed', 1);
                 INSERT INTO messages (id, agent_id, turn_id, group_id, author_agent_id, role, content, created_at)
                 VALUES ('m1', 'a', 't', 'g', NULL, 'user', 'hi', 1),
                        ('m2', 'a', 't', 'g', 'a', 'assistant', 'hello', 2);
                 INSERT INTO steps VALUES ('s', 't', 0, 'tool', 'x', NULL, 'done', 1);
                 INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
                 VALUES ('q', 'g', 'a', 'later', 0, 1);
                 INSERT INTO agent_group_sessions (agent_id, group_id, provider_session_id, last_seen_seq) VALUES ('a', 'g', 'session', 2), ('b', 'g', NULL, 0);",
            )
            .unwrap()
        });
    }

    #[test]
    fn deleting_a_group_removes_its_turns_messages_steps_queue_and_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        seed_group_chat(&db);
        db.with_connection(|c| c.execute("DELETE FROM groups WHERE id = 'g'", []).unwrap());
        for table in [
            "turns",
            "messages",
            "steps",
            "group_queue",
            "agent_group_sessions",
            "group_members",
        ] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
        assert_eq!(count(&db, "agents"), 2);
    }

    fn version_five_database(dir: &Path) {
        let mut connection = Connection::open(dir.join(DATABASE_FILE)).unwrap();
        configure(&connection).unwrap();
        let sources = [
            (
                "V1__initial",
                include_str!("../../migrations/V1__initial.sql"),
            ),
            (
                "V2__settings",
                include_str!("../../migrations/V2__settings.sql"),
            ),
            (
                "V3__auth_mode",
                include_str!("../../migrations/V3__auth_mode.sql"),
            ),
            (
                "V4__groups",
                include_str!("../../migrations/V4__groups.sql"),
            ),
            (
                "V5__group_turns",
                include_str!("../../migrations/V5__group_turns.sql"),
            ),
        ];
        let v5: Vec<_> = sources
            .into_iter()
            .map(|(name, sql)| refinery::Migration::unapplied(name, sql).unwrap())
            .collect();
        migrate(&mut connection, Runner::new(&v5)).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/a', 1, 1),
                        ('b', 'Bo', 'blue', 'claude-code', 'm', '/b', 1, 1);
                 INSERT INTO groups VALUES ('g', 'Team', '/g', 'a', 1, 1);
                 INSERT INTO group_members VALUES ('g', 'a', 0, 1), ('g', 'b', 1, 1);
                 INSERT INTO turns (id, agent_id, group_id, status, error, started_at, ended_at)
                 VALUES ('t1', 'a', NULL, 'completed', NULL, 1, 2),
                        ('t2', 'a', 'g', 'failed', 'boom', 3, 4),
                        ('t3', 'b', 'g', 'completed', NULL, 5, 6);
                 INSERT INTO messages (id, agent_id, turn_id, group_id, author_agent_id, role, content, created_at)
                 VALUES ('m1', 'a', 't1', NULL, NULL, 'user', 'private', 1),
                        ('m2', 'a', 't2', 'g', NULL, 'user', 'to the group', 3),
                        ('m3', 'a', 't2', 'g', 'a', 'assistant', 'answer', 4),
                        ('m4', 'b', 't3', 'g', 'b', 'assistant', 'from bo', 5);
                 INSERT INTO steps VALUES ('s', 't2', 0, 'tool', 'x', NULL, 'done', 3);
                 INSERT INTO queued_messages VALUES ('q', 'a', 'later', 0, 1);
                 INSERT INTO group_queue (id, group_id, addressed_agent_id, content, position, created_at)
                 VALUES ('gq', 'g', 'b', 'for bo', 0, 1);
                 INSERT INTO agent_group_sessions VALUES ('a', 'g', 'sess', 3), ('b', 'g', NULL, 4);",
            )
            .unwrap();
    }

    #[test]
    fn the_group_history_migration_keeps_one_to_one_and_group_rows_of_a_version_five_database() {
        let dir = tempfile::tempdir().unwrap();
        version_five_database(dir.path());
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 7);
        for (table, rows) in [
            ("agents", 2),
            ("turns", 3),
            ("messages", 4),
            ("steps", 1),
            ("queued_messages", 1),
            ("groups", 1),
            ("group_members", 2),
            ("group_queue", 1),
            ("agent_group_sessions", 2),
        ] {
            assert_eq!(count(&db, table), rows, "{table}");
        }
        type TurnRow = (
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        );
        let turns: Vec<TurnRow> = db.with_connection(|c| {
            c.prepare("SELECT id, agent_id, group_id, status, error FROM turns ORDER BY rowid")
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        });
        assert_eq!(
            turns,
            [
                (
                    "t1".into(),
                    Some("a".into()),
                    None,
                    "completed".into(),
                    None
                ),
                (
                    "t2".into(),
                    Some("a".into()),
                    Some("g".into()),
                    "failed".into(),
                    Some("boom".into())
                ),
                (
                    "t3".into(),
                    Some("b".into()),
                    Some("g".into()),
                    "completed".into(),
                    None
                ),
            ]
        );
        let messages: Vec<(String, Option<String>, Option<String>, i64)> =
            db.with_connection(|c| {
                c.prepare("SELECT id, group_id, author_agent_id, seq FROM messages ORDER BY rowid")
                    .unwrap()
                    .query_map([], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap()
            });
        assert_eq!(
            messages,
            [
                ("m1".into(), None, None, 1),
                ("m2".into(), Some("g".into()), None, 2),
                ("m3".into(), Some("g".into()), Some("a".into()), 3),
                ("m4".into(), Some("g".into()), Some("b".into()), 4),
            ]
        );
        let seen: Vec<i64> = db.with_connection(|c| {
            c.prepare("SELECT last_seen_seq FROM agent_group_sessions ORDER BY agent_id")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        });
        assert_eq!(seen, [3, 4]);
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO messages (id, agent_id, turn_id, role, content, created_at)
                 VALUES ('m5', 'a', 't1', 'assistant', 'new', 9);
                 DELETE FROM messages WHERE id = 'm5';
                 INSERT INTO messages (id, agent_id, turn_id, role, content, created_at)
                 VALUES ('m6', 'a', 't1', 'assistant', 'newer', 10);",
            )
            .unwrap()
        });
        let newest: i64 = db.with_connection(|c| {
            c.query_row("SELECT seq FROM messages WHERE id = 'm6'", [], |row| {
                row.get(0)
            })
            .unwrap()
        });
        assert_eq!(newest, 6);
        db.with_connection(|c| c.execute("DELETE FROM agents WHERE id = 'b'", []).unwrap());
        let (turn_agent, message_agent, author): (Option<String>, Option<String>, Option<String>) =
            db.with_connection(|c| {
                c.query_row(
                    "SELECT t.agent_id, m.agent_id, m.author_agent_id
                     FROM turns t JOIN messages m ON m.turn_id = t.id WHERE m.id = 'm4'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
            });
        assert_eq!((turn_agent, message_agent, author), (None, None, None));
        assert_eq!(count(&db, "messages"), 5);
        let unhashed: i64 = db.with_connection(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM agent_group_sessions WHERE context_hash IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap()
        });
        assert_eq!(unhashed, 1);
        let indexes: i64 = db.with_connection(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN
                 ('turns_agent_started', 'turns_group_started', 'messages_agent_created', 'messages_group')",
                [],
                |row| row.get(0),
            )
            .unwrap()
        });
        assert_eq!(indexes, 4);
        let violations: i64 = db.with_connection(|c| {
            c.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .mapped(|_| Ok(()))
                .count() as i64
        });
        assert_eq!(violations, 0);
    }

    #[test]
    fn deleting_an_agent_removes_its_memberships_and_clears_a_lead_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('a', 'Ada', 'red', 'claude-code', 'm', '/a', 1, 1),
                        ('b', 'Bo', 'red', 'claude-code', 'm', '/b', 1, 1);
                 INSERT INTO groups VALUES ('g', 'Team', '/g', 'a', 1, 1);
                 INSERT INTO group_members VALUES ('g', 'a', 0, 1), ('g', 'b', 1, 1);",
            )
            .unwrap();
            c.execute("DELETE FROM agents WHERE id = 'a'", []).unwrap();
        });
        assert_eq!(count(&db, "group_members"), 1);
        assert_eq!(count(&db, "groups"), 1);
        let lead: Option<String> = db.with_connection(|c| {
            c.query_row("SELECT lead_agent_id FROM groups", [], |row| row.get(0))
                .unwrap()
        });
        assert_eq!(lead, None);
        db.with_connection(|c| c.execute("DELETE FROM groups", []).unwrap());
        assert_eq!(count(&db, "group_members"), 0);
        assert_eq!(count(&db, "agents"), 1);
    }

    #[test]
    fn group_names_are_unique_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let insert = |id: &str, name: &str| {
            db.with_connection(|c| {
                c.execute(
                    "INSERT INTO groups VALUES (?1, ?2, '/g', NULL, 1, 1)",
                    [id, name],
                )
            })
        };
        assert!(insert("1", "Team").is_ok());
        assert!(insert("2", "TEAM").is_err());
    }

    #[test]
    fn the_agent_auth_mode_column_refuses_an_unknown_value() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let insert = |mode: &str| {
            db.with_connection(|c| {
                c.execute(
                    "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at, auth_mode)
                     VALUES (?1, ?1, 'red', 'claude-code', 'm', '/w', 1, 1, ?2)",
                    [mode, mode],
                )
            })
        };
        assert!(insert("api-key").is_ok());
        assert!(insert("subscription").is_ok());
        assert!(insert("oauth").is_err());
    }
}
