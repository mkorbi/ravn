//! Skills-table CRUD primitives. The high-level FS-canonical + DB-mirror
//! sync logic lives in `crates/skills` — this module is the storage layer
//! only.

use serde::{Deserialize, Serialize};

use crate::db::{now_millis, Db};
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SkillRow {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub body: String,
    /// JSON-encoded `Vec<String>` — caller parses as needed.
    pub trigger_patterns: String,
    /// JSON-encoded `Vec<String>` — caller parses as needed.
    pub allowed_tools: String,
    pub body_hash: String,
    pub fs_path: String,
    pub indexed_at: i64,
}

pub async fn get_by_name(db: &Db, name: &str) -> Result<Option<SkillRow>, Error> {
    let row = sqlx::query_as::<_, SkillRow>("SELECT * FROM skills WHERE name = ?1")
        .bind(name)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row)
}

pub async fn list_all(db: &Db) -> Result<Vec<SkillRow>, Error> {
    let rows = sqlx::query_as::<_, SkillRow>("SELECT * FROM skills ORDER BY name")
        .fetch_all(&db.pool)
        .await?;
    Ok(rows)
}

/// Insert a brand-new skill row. Returns the rowid (matches
/// `skills_vec.rowid` for the per-skill embedding).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    db: &Db,
    name: &str,
    description: &str,
    body: &str,
    trigger_patterns_json: &str,
    allowed_tools_json: &str,
    body_hash: &str,
    fs_path: &str,
) -> Result<i64, Error> {
    let id = sqlx::query(
        r#"INSERT INTO skills
            (name, description, body, trigger_patterns, allowed_tools, body_hash, fs_path, indexed_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
    )
    .bind(name)
    .bind(description)
    .bind(body)
    .bind(trigger_patterns_json)
    .bind(allowed_tools_json)
    .bind(body_hash)
    .bind(fs_path)
    .bind(now_millis())
    .execute(&db.pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// Update an existing skill in place — preserves the rowid so the
/// linked `skills_vec` row stays valid.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    db: &Db,
    id: i64,
    description: &str,
    body: &str,
    trigger_patterns_json: &str,
    allowed_tools_json: &str,
    body_hash: &str,
    fs_path: &str,
) -> Result<(), Error> {
    let now = now_millis();
    sqlx::query(
        r#"UPDATE skills
              SET description = ?1, body = ?2, trigger_patterns = ?3,
                  allowed_tools = ?4, body_hash = ?5, fs_path = ?6,
                  indexed_at = ?7
            WHERE id = ?8"#,
    )
    .bind(description)
    .bind(body)
    .bind(trigger_patterns_json)
    .bind(allowed_tools_json)
    .bind(body_hash)
    .bind(fs_path)
    .bind(now)
    .bind(id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn delete_by_name(db: &Db, name: &str) -> Result<(), Error> {
    sqlx::query("DELETE FROM skills WHERE name = ?1")
        .bind(name)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// FTS5 search over name + description + body. Returns rows ranked by BM25.
pub async fn search(db: &Db, query: &str, limit: i64) -> Result<Vec<SkillRow>, Error> {
    let rows = sqlx::query_as::<_, SkillRow>(
        r#"SELECT s.*
             FROM skills_fts
             JOIN skills s ON s.id = skills_fts.rowid
            WHERE skills_fts MATCH ?1
         ORDER BY rank
            LIMIT ?2"#,
    )
    .bind(query)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows)
}

/// Fetch a set of skills by their row IDs. Preserves the input order
/// — used by [`search_hybrid`] to return rows in RRF-ranked order.
pub async fn get_by_ids(db: &Db, ids: &[i64]) -> Result<Vec<SkillRow>, Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT * FROM skills WHERE id IN ({placeholders})");
    let mut q = sqlx::query_as::<_, SkillRow>(&sql);
    for id in ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(&db.pool).await?;
    let mut by_id: std::collections::HashMap<i64, SkillRow> =
        rows.into_iter().map(|r| (r.id, r)).collect();
    Ok(ids.iter().filter_map(|i| by_id.remove(i)).collect())
}

/// Hybrid FTS5 + vector search with Reciprocal Rank Fusion. Mirrors
/// [`crate::messages::search_hybrid`] but against the `skills` table +
/// `skills_vec` virtual table. Returns up to `limit` rows ranked by
/// combined RRF score (k_rrf=60).
pub async fn search_hybrid(
    db: &Db,
    query_text: &str,
    query_vec: &[f32],
    limit: i64,
) -> Result<Vec<SkillRow>, Error> {
    const K_RRF: f64 = 60.0;
    let over = (limit * 2).max(20);

    if query_vec.is_empty() {
        return search(db, query_text, limit).await;
    }

    let (text_res, vec_res) = tokio::join!(
        search(db, query_text, over),
        crate::vector::search(db, crate::vector::VecTable::Skills, query_vec, over),
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
