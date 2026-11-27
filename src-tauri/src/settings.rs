use rusqlite::{Connection, OptionalExtension};

pub fn get(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
}

pub fn set(connection: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

pub fn delete(connection: &Connection, key: &str) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM settings WHERE key = ?1", [key])?;
    Ok(())
}
