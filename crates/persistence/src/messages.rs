use serde::{Deserialize, Serialize};

use crate::db::{now_millis, Db};
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MessageRow {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    /// JSON-encoded `Vec<ContentBlock>` from the LLM crate.
    pub content: String,
    pub created_at: i64,
}

pub async fn append(
    db: &Db,
    session_id: &str,
    role: &str,
    content_json: &str,
) -> Result<i64, Error> {
    let now = now_millis();
    let id = sqlx::query(
        r#"INSERT INTO messages (session_id, role, content, created_at)
           VALUES (?1, ?2, ?3, ?4)"#,
    )
    .bind(session_id)
    .bind(role)
    .bind(content_json)
    .bind(now)
    .execute(&db.pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn list_session(db: &Db, session_id: &str) -> Result<Vec<MessageRow>, Error> {
    let rows = sqlx::query_as::<_, MessageRow>(
        "SELECT id, session_id, role, content, created_at
           FROM messages
          WHERE session_id = ?1
       ORDER BY created_at ASC, id ASC",
    )
    .bind(session_id)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows)
}

/// FTS5 search across all sessions. Returns rows ranked by BM25.
pub async fn search(db: &Db, query: &str, limit: i64) -> Result<Vec<MessageRow>, Error> {
    let rows = sqlx::query_as::<_, MessageRow>(
        r#"SELECT m.id, m.session_id, m.role, m.content, m.created_at
             FROM messages_fts
             JOIN messages m ON m.id = messages_fts.rowid
            WHERE messages_fts MATCH ?1
         ORDER BY rank
            LIMIT ?2"#,
    )
    .bind(query)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows)
}
