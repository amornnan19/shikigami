use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("could not create the data folder {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not open the database {path}: {source}")]
    Open {
        path: PathBuf,
        source: rusqlite::Error,
    },
    #[error("could not set up the database: {0}")]
    Pragma(rusqlite::Error),
    #[error("the database has {0} broken links between records after the update")]
    ForeignKeyViolations(usize),
    #[error("could not update the database: {0}")]
    Migrate(#[from] refinery::Error),
}
