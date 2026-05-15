use serde::{Deserialize, Serialize};

use crate::db::{now_millis, Db};
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub channel: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_usd: f64,
}

/// Incremental usage update applied after each LLM call.
#[derive(Debug, Clone, Copy, Default)]
pub struct UsageDelta {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    pub reasoning_tokens: u32,
    pub cost_usd: f64,
}

pub async fn create(
    db: &Db,
    id: &str,
    channel: &str,
    model: Option<&str>,
) -> Result<Session, Error> {
    let started_at = now_millis();
    sqlx::query(
        r#"INSERT INTO sessions (id, started_at, channel, model)
           VALUES (?1, ?2, ?3, ?4)"#,
    )
    .bind(id)
    .bind(started_at)
    .bind(channel)
    .bind(model)
    .execute(&db.pool)
    .await?;

    get(db, id)
        .await?
        .ok_or_else(|| Error::SessionNotFound(id.to_string()))
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Session>, Error> {
    let row = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = ?1")
        .bind(id)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row)
}

pub async fn close(db: &Db, id: &str) -> Result<(), Error> {
    let now = now_millis();
    sqlx::query("UPDATE sessions SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Apply an incremental usage delta to a session. Called after every LLM
/// completion to keep the running totals current. Idempotent against the
/// specific delta — caller must ensure each completion is reported once.
pub async fn bump_usage(db: &Db, id: &str, delta: UsageDelta) -> Result<(), Error> {
    sqlx::query(
        r#"UPDATE sessions
              SET input_tokens          = input_tokens          + ?1,
                  output_tokens         = output_tokens         + ?2,
                  cache_read_tokens     = cache_read_tokens     + ?3,
                  cache_creation_tokens = cache_creation_tokens + ?4,
                  reasoning_tokens      = reasoning_tokens      + ?5,
                  cost_usd              = cost_usd              + ?6
            WHERE id = ?7"#,
    )
    .bind(delta.input_tokens as i64)
    .bind(delta.output_tokens as i64)
    .bind(delta.cache_read_tokens as i64)
    .bind(delta.cache_creation_tokens as i64)
    .bind(delta.reasoning_tokens as i64)
    .bind(delta.cost_usd)
    .bind(id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn recent(db: &Db, limit: i64) -> Result<Vec<Session>, Error> {
    let rows = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions ORDER BY started_at DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows)
}
