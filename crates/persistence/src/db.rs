use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::Error;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Handle to the agent's SQLite database. Cheap to clone (`SqlitePool` is an `Arc` inside).
#[derive(Clone)]
pub struct Db {
    pub(crate) pool: SqlitePool,
    /// Canonical path on disk — `:memory:` for the in-memory variant.
    /// Used by [`crate::vector`] to open separate `rusqlite` connections
    /// for the `sqlite-vec`-backed `vec0` tables.
    pub(crate) path: PathBuf,
}

impl Db {
    /// Open (or create) the database at `path`, set WAL+pragmas, run
    /// pending sqlx migrations, and bootstrap the `sqlite-vec` `vec0`
    /// tables ([`crate::vector`]).
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let url = path
            .to_str()
            .ok_or_else(|| Error::InvalidPath(path.display().to_string()))?;

        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{url}"))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;

        MIGRATOR.run(&pool).await?;

        let db = Self { pool, path };
        crate::vector::bootstrap(&db).await?;
        Ok(db)
    }

    /// Open an in-memory database (for tests). Vector operations are
    /// **not** supported here — rusqlite can't share the same in-memory
    /// SQLite instance as sqlx. Use a tempfile path with [`open`] for
    /// tests that need `vec0`.
    pub async fn open_in_memory() -> Result<Self, Error> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?
            .journal_mode(SqliteJournalMode::Memory)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;

        MIGRATOR.run(&pool).await?;

        Ok(Self {
            pool,
            path: PathBuf::from(":memory:"),
        })
    }

    /// Underlying connection pool, for callers that need raw `sqlx` access.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

/// Helper for callers that produce Unix-millis timestamps.
pub fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
