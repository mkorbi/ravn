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

/// Fetch a set of messages by their row IDs. Preserves the input order
/// — used by [`search_hybrid`] to return rows in RRF-ranked order.
pub async fn get_by_ids(db: &Db, ids: &[i64]) -> Result<Vec<MessageRow>, Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // Build `WHERE id IN (?, ?, ?)` dynamically — sqlx doesn't expand
    // slices on its own.
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, session_id, role, content, created_at FROM messages WHERE id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, MessageRow>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(&db.pool).await?;
    // Re-sort by the input id order.
    let mut by_id: std::collections::HashMap<i64, MessageRow> =
        rows.into_iter().map(|r| (r.id, r)).collect();
    Ok(ids.iter().filter_map(|i| by_id.remove(i)).collect())
}

/// Hybrid FTS5 + vector search with Reciprocal Rank Fusion (Phase 2.10).
///
/// Runs the FTS5 (BM25) and `sqlite-vec` k-NN queries concurrently, then
/// merges via RRF: `score(d) = Σ 1 / (k_rrf + rank_i(d))`. Each result
/// list is overfetched 2× before merging so the final top-N is a
/// stable union.
///
/// `query_vec` must have length [`crate::vector::EMBEDDING_DIM`] (1024).
/// Falls back to FTS5-only if the caller passes an empty vector.
pub async fn search_hybrid(
    db: &Db,
    query_text: &str,
    query_vec: &[f32],
    limit: i64,
) -> Result<Vec<MessageRow>, Error> {
    const K_RRF: f64 = 60.0;
    let over = (limit * 2).max(20);

    if query_vec.is_empty() {
        return search(db, query_text, limit).await;
    }

    let (text_res, vec_res) = tokio::join!(
        search(db, query_text, over),
        crate::vector::search(db, crate::vector::VecTable::Messages, query_vec, over),
    );
    let text_hits = text_res?;
    let vec_hits = vec_res?;

    let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    for (rank, row) in text_hits.iter().enumerate() {
        *scores.entry(row.id).or_default() += 1.0 / (K_RRF + (rank + 1) as f64);
    }
    for (rank, hit) in vec_hits.iter().enumerate() {
        *scores.entry(hit.rowid).or_default() += 1.0 / (K_RRF + (rank + 1) as f64);
    }

    let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_ids: Vec<i64> = ranked
        .into_iter()
        .take(limit as usize)
        .map(|(id, _)| id)
        .collect();

    get_by_ids(db, &top_ids).await
}
