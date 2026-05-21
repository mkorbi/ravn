//! Cross-session tool-approval allowlist (Phase 2.11, D13).
//!
//! `TuiApprover` reads this table at startup into an in-memory
//! `HashSet`, and writes back when the user resolves an approval modal
//! with `a` (`AllowAndRemember`). Pure tool-name match — no args
//! pattern. Removal via [`remove`] or direct SQL.

use crate::db::{now_millis, Db};
use crate::error::Error;

pub async fn list_all(db: &Db) -> Result<Vec<String>, Error> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT tool_name FROM tool_allowlist ORDER BY created_at")
            .fetch_all(&db.pool)
            .await?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

pub async fn insert(db: &Db, tool_name: &str) -> Result<(), Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO tool_allowlist (tool_name, created_at) VALUES (?1, ?2)",
    )
    .bind(tool_name)
    .bind(now_millis())
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn remove(db: &Db, tool_name: &str) -> Result<(), Error> {
    sqlx::query("DELETE FROM tool_allowlist WHERE tool_name = ?1")
        .bind(tool_name)
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn contains(db: &Db, tool_name: &str) -> Result<bool, Error> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM tool_allowlist WHERE tool_name = ?1 LIMIT 1")
            .bind(tool_name)
            .fetch_optional(&db.pool)
            .await?;
    Ok(row.is_some())
}
