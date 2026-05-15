#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("encoding: {0}")]
    Encoding(#[from] serde_json::Error),

    #[error("session {0} not found")]
    SessionNotFound(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),
}
