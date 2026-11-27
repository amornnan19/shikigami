mod error;

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

pub use error::DbError;

mod embedded {
    refinery::embed_migrations!("migrations");
}

pub const DATABASE_FILE: &str = "shikigami.db";

pub struct Database {
    connection: Mutex<Connection>,
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
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;",
            )
            .map_err(DbError::Pragma)?;
        embedded::migrations::runner().run(&mut connection)?;
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
                "INSERT INTO agents VALUES ('a', 'Ada', 'red', 'codex', 'm', '/w', NULL, 1, 1);
                 INSERT INTO turns VALUES ('t', 'a', 'running', NULL, 1, NULL);
                 INSERT INTO messages VALUES ('m', 'a', 't', 'user', 'hi', 1);
                 INSERT INTO steps VALUES ('s', 't', 0, 'tool', 'x', NULL, 'running', 1);
                 INSERT INTO queued_messages VALUES ('q', 'a', 'later', 0, 1);",
            )
            .unwrap()
        })
    }

    #[test]
    fn fresh_database_migrates_to_version_one_with_all_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 1);
        for table in ["agents", "turns", "messages", "steps", "queued_messages"] {
            assert_eq!(count(&db, table), 0, "{table}");
        }
    }

    #[test]
    fn opening_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        drop(Database::open(dir.path()).unwrap());
        let db = Database::open(dir.path()).unwrap();
        assert_eq!(schema_version(&db), 1);
        assert_eq!(count(&db, "refinery_schema_history"), 1);
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
        assert!(result.is_err());
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
                "V2__broken",
                "CREATE TABLE probe (x INTEGER); INSERT INTO missing_table VALUES (1);",
            )
            .unwrap(),
        ];
        let result = refinery::Runner::new(&migrations).run(&mut connection);
        assert!(result.is_err());
        let version: u32 = connection
            .query_row(
                "SELECT MAX(version) FROM refinery_schema_history",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
        let probe_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'probe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(probe_tables, 0);
    }
}
