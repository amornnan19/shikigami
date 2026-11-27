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
                 INSERT INTO turns VALUES ('t', 'a', 'running', NULL, 1, NULL);
                 INSERT INTO messages VALUES ('m', 'a', 't', 'user', 'hi', 1);
                 INSERT INTO steps VALUES ('s', 't', 0, 'tool', 'x', NULL, 'running', 1);
                 INSERT INTO queued_messages VALUES ('q', 'a', 'later', 0, 1);",
            )
            .unwrap()
        })
    }

    #[test]
    fn fresh_database_migrates_to_the_latest_version_with_all_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 3);
        for table in [
            "agents",
            "turns",
            "messages",
            "steps",
            "queued_messages",
            "settings",
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
        assert_eq!(schema_version(&db), 3);
        assert_eq!(count(&db, "agents"), 1);
        assert_eq!(count(&db, "turns"), 1);
        assert_eq!(count(&db, "settings"), 0);
    }

    #[test]
    fn opening_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        drop(Database::open(dir.path()).unwrap());
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 3);
        assert_eq!(count(&db, "refinery_schema_history"), 3);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        let result = db.with_connection(|c| {
            c.execute(
                "INSERT INTO turns VALUES ('t', 'missing', 'running', NULL, 1, NULL)",
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
        db.with_connection(|c| c.execute("DELETE FROM agents WHERE id = 'a'", []).unwrap());
        for table in ["turns", "messages", "steps", "queued_messages"] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
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
                "V4__broken",
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
        assert_eq!(version, 3);
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
                "V4__rebuild_agents",
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
        assert_eq!(schema_version(&db), 4);
        for table in ["agents", "turns", "messages", "steps", "queued_messages"] {
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
            assert_eq!(schema_version(&db), 3);
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
