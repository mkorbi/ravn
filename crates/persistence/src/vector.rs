//! `sqlite-vec` integration (Phase 2.9, D3 + D12).
//!
//! `sqlite-vec` is a SQLite extension that adds the `vec0` virtual table
//! type. We load it once per process via `sqlite3_auto_extension`; every
//! `rusqlite::Connection::open` after that auto-loads the extension.
//! sqlx connections in `Db::pool` never touch `vec0` (sqlx doesn't know
//! about the extension), so they don't need it loaded.
//!
//! The `vec0` tables (`messages_vec`, `skills_vec`) are bootstrapped
//! idempotent from [`Db::open`]. They're not part of the sqlx
//! migrations folder because sqlx wouldn't know how to read them
//! anyway (no `CREATE VIRTUAL TABLE ... USING vec0` without the
//! extension loaded).
//!
//! Dim is hardcoded to 1024 to match `ravn_embeddings::EMBEDDING_DIM`
//! ([D12]: Qwen3-Embedding-0.6B). If you ever swap models, update both
//! here and in the embeddings crate.

use std::sync::Once;

use rusqlite::Connection;

use crate::db::Db;
use crate::error::Error;

/// Must match `ravn_embeddings::EMBEDDING_DIM`. Asserted at insert time.
pub const EMBEDDING_DIM: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VecTable {
    Messages,
    Skills,
}

impl VecTable {
    fn name(self) -> &'static str {
        match self {
            VecTable::Messages => "messages_vec",
            VecTable::Skills => "skills_vec",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VecHit {
    pub rowid: i64,
    pub distance: f32,
}

static INIT: Once = Once::new();

/// Register the `sqlite-vec` extension globally. Called transparently by
/// every public function in this module; safe to call repeatedly.
fn ensure_extension_registered() {
    INIT.call_once(|| {
        // SAFETY: the sqlite-vec crate exposes its entry point as a
        // signature-less `fn()` symbol, but SQLite expects the real
        // extension-init signature. We cast through `*const ()` to
        // re-type the function pointer. This is the pattern documented
        // in sqlite-vec's own tests.
        unsafe {
            let entry: unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int =
                std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(entry));
        }
    });
}

/// Idempotent `CREATE VIRTUAL TABLE IF NOT EXISTS` for both vec tables.
/// Called from [`Db::open`]; no-op for `:memory:` (rusqlite can't share
/// the sqlx in-memory DB, so vec ops are unsupported there).
pub(crate) async fn bootstrap(db: &Db) -> Result<(), Error> {
    let path = db.path.clone();
    if path.as_os_str() == ":memory:" {
        return Ok(());
    }
    tokio::task::spawn_blocking(move || -> Result<(), Error> {
        ensure_extension_registered();
        let conn = Connection::open(&path)?;
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_vec USING vec0(embedding float[{dim}]);
             CREATE VIRTUAL TABLE IF NOT EXISTS skills_vec   USING vec0(embedding float[{dim}]);",
            dim = EMBEDDING_DIM
        ))?;
        Ok(())
    })
    .await
    .map_err(|e| Error::Join(e.to_string()))?
}

/// Insert (or replace) one embedding row for `table` at `rowid`.
pub async fn insert(
    db: &Db,
    table: VecTable,
    rowid: i64,
    embedding: &[f32],
) -> Result<(), Error> {
    if embedding.len() != EMBEDDING_DIM {
        return Err(Error::WrongDim {
            expected: EMBEDDING_DIM,
            actual: embedding.len(),
        });
    }
    let path = db.path.clone();
    let blob = embedding_to_blob(embedding);
    tokio::task::spawn_blocking(move || -> Result<(), Error> {
        ensure_extension_registered();
        let conn = Connection::open(&path)?;
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO {} (rowid, embedding) VALUES (?1, ?2)",
                table.name()
            ),
            rusqlite::params![rowid, blob],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| Error::Join(e.to_string()))?
}

/// Insert many embeddings in one transaction.
pub async fn insert_batch(
    db: &Db,
    table: VecTable,
    items: Vec<(i64, Vec<f32>)>,
) -> Result<(), Error> {
    for (i, (_, v)) in items.iter().enumerate() {
        if v.len() != EMBEDDING_DIM {
            return Err(Error::WrongDim {
                expected: EMBEDDING_DIM,
                actual: v.len(),
            });
        }
        let _ = i;
    }
    let path = db.path.clone();
    let payload: Vec<(i64, Vec<u8>)> = items
        .into_iter()
        .map(|(id, v)| (id, embedding_to_blob(&v)))
        .collect();
    tokio::task::spawn_blocking(move || -> Result<(), Error> {
        ensure_extension_registered();
        let mut conn = Connection::open(&path)?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(&format!(
                "INSERT OR REPLACE INTO {} (rowid, embedding) VALUES (?1, ?2)",
                table.name()
            ))?;
            for (rowid, blob) in &payload {
                stmt.execute(rusqlite::params![rowid, blob])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .map_err(|e| Error::Join(e.to_string()))?
}

/// k-NN search: return the `k` rowids closest to `query` plus their
/// distance score (smaller = closer; sqlite-vec uses L2 by default).
pub async fn search(
    db: &Db,
    table: VecTable,
    query: &[f32],
    k: i64,
) -> Result<Vec<VecHit>, Error> {
    if query.len() != EMBEDDING_DIM {
        return Err(Error::WrongDim {
            expected: EMBEDDING_DIM,
            actual: query.len(),
        });
    }
    let path = db.path.clone();
    let blob = embedding_to_blob(query);
    tokio::task::spawn_blocking(move || -> Result<Vec<VecHit>, Error> {
        ensure_extension_registered();
        let conn = Connection::open(&path)?;
        let mut stmt = conn.prepare(&format!(
            "SELECT rowid, distance FROM {} WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2",
            table.name()
        ))?;
        let rows = stmt
            .query_map(rusqlite::params![blob, k], |row| {
                Ok(VecHit {
                    rowid: row.get(0)?,
                    distance: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
    .map_err(|e| Error::Join(e.to_string()))?
}

/// Delete one row by `rowid`.
pub async fn delete(db: &Db, table: VecTable, rowid: i64) -> Result<(), Error> {
    let path = db.path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), Error> {
        ensure_extension_registered();
        let conn = Connection::open(&path)?;
        conn.execute(
            &format!("DELETE FROM {} WHERE rowid = ?1", table.name()),
            rusqlite::params![rowid],
        )?;
        Ok(())
    })
    .await
    .map_err(|e| Error::Join(e.to_string()))?
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for f in embedding {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn vec_db() -> (Db, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vec.db");
        let db = Db::open(&path).await.unwrap();
        (db, dir)
    }

    fn unit_vec(seed: f32) -> Vec<f32> {
        // Cheap deterministic test vector — fill with a constant, then
        // normalize to unit length so distances are comparable.
        let raw: Vec<f32> = (0..EMBEDDING_DIM).map(|i| (i as f32 * 0.001) + seed).collect();
        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        raw.into_iter().map(|x| x / norm).collect()
    }

    #[tokio::test]
    async fn bootstrap_creates_vec_tables() {
        let (db, _td) = vec_db().await;
        // Inserting must work; no "no such table" error.
        insert(&db, VecTable::Messages, 1, &unit_vec(0.0))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn insert_then_search_returns_inserted_row_first() {
        let (db, _td) = vec_db().await;

        let v1 = unit_vec(0.0);
        let v2 = unit_vec(1.0);
        let v3 = unit_vec(2.0);
        insert(&db, VecTable::Messages, 1, &v1).await.unwrap();
        insert(&db, VecTable::Messages, 2, &v2).await.unwrap();
        insert(&db, VecTable::Messages, 3, &v3).await.unwrap();

        // Querying with v2 should rank rowid=2 as closest.
        let hits = search(&db, VecTable::Messages, &v2, 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].rowid, 2);
    }

    #[tokio::test]
    async fn insert_batch_then_search() {
        let (db, _td) = vec_db().await;
        let items = vec![
            (10, unit_vec(0.0)),
            (11, unit_vec(0.5)),
            (12, unit_vec(2.0)),
        ];
        insert_batch(&db, VecTable::Skills, items).await.unwrap();
        let hits = search(&db, VecTable::Skills, &unit_vec(0.5), 2).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].rowid, 11);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let (db, _td) = vec_db().await;
        insert(&db, VecTable::Messages, 7, &unit_vec(0.0))
            .await
            .unwrap();
        delete(&db, VecTable::Messages, 7).await.unwrap();
        let hits = search(&db, VecTable::Messages, &unit_vec(0.0), 5)
            .await
            .unwrap();
        assert!(hits.iter().all(|h| h.rowid != 7));
    }

    #[tokio::test]
    async fn wrong_dim_rejected() {
        let (db, _td) = vec_db().await;
        let err = insert(&db, VecTable::Messages, 1, &[0.1, 0.2, 0.3])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::WrongDim { expected: 1024, actual: 3 }));
    }
}
