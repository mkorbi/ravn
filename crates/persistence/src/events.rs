use serde::Serialize;

use crate::db::{now_millis, Db};
use crate::error::Error;

/// Append-only event for tracing, trajectory logs, and audit.
///
/// `payload` is BLOB in the schema. Phase 0 encodes as UTF-8 JSON via
/// [`append_json`]; Phase 1 will switch to postcard binary without changing
/// the column type.
pub async fn append(
    db: &Db,
    trace_id: Option<&str>,
    session_id: Option<&str>,
    kind: &str,
    payload: &[u8],
) -> Result<i64, Error> {
    let now = now_millis();
    let id = sqlx::query(
        r#"INSERT INTO events (trace_id, session_id, kind, payload, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
    )
    .bind(trace_id)
    .bind(session_id)
    .bind(kind)
    .bind(payload)
    .bind(now)
    .execute(&db.pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn append_json<T: Serialize>(
    db: &Db,
    trace_id: Option<&str>,
    session_id: Option<&str>,
    kind: &str,
    payload: &T,
) -> Result<i64, Error> {
    let bytes = serde_json::to_vec(payload)?;
    append(db, trace_id, session_id, kind, &bytes).await
}

pub async fn count(db: &Db) -> Result<i64, Error> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(&db.pool)
        .await?;
    Ok(n)
}
