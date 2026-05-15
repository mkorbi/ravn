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
