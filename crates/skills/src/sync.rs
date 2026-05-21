//! Filesystem → DB sync (Phase 2.4, D11).
//!
//! Walks the `Vec<Skill>` from [`crate::load_all_from_fs`] and upserts
//! each into the `skills` DB mirror. Unchanged bodies (matching SHA-256
//! body hash) are left alone; new or modified bodies trigger a
//! fire-and-forget embedding into `skills_vec` (which Phase 2.5's
//! Top-K matcher will read).
//!
//! Skills present in the DB but no longer on disk are deleted (the
//! filesystem is canonical per D11).

use std::collections::HashSet;
use std::sync::Arc;

use ravn_embeddings::Embedder;
use ravn_persistence::{skills as skills_db, vector, Db};
use sha2::{Digest, Sha256};

use crate::parser::Skill;
use crate::Error;

#[derive(Debug, Clone, Copy, Default)]
pub struct SyncStats {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
}

/// Reconcile the DB mirror with the in-memory `skills` list. Returns
/// counts so the caller can log a summary.
///
/// If `embedder` is `Some`, new/updated skills also get a
/// fire-and-forget embedding into `skills_vec`. Pass `None` to skip
/// the index update (useful in tests).
pub async fn sync_to_db(
    db: &Db,
    skills: Vec<Skill>,
    embedder: Option<Arc<Embedder>>,
) -> Result<SyncStats, Error> {
    let mut stats = SyncStats::default();
    let fs_names: HashSet<String> = skills.iter().map(|s| s.name.clone()).collect();

    for skill in skills {
        let trigger_json =
            serde_json::to_string(&skill.trigger_patterns).unwrap_or_else(|_| "[]".into());
        let allowed_json =
            serde_json::to_string(&skill.allowed_tools).unwrap_or_else(|_| "[]".into());
        let body_hash = hex_sha256(&skill.body);
        let fs_path_str = skill.fs_path.display().to_string();

        let existing = skills_db::get_by_name(db, &skill.name).await?;
        let (rowid, embed_required) = match existing {
            Some(row) if row.body_hash == body_hash => {
                stats.unchanged += 1;
                (row.id, false)
            }
            Some(row) => {
                skills_db::update(
                    db,
                    row.id,
                    &skill.description,
                    &skill.body,
                    &trigger_json,
                    &allowed_json,
                    &body_hash,
                    &fs_path_str,
                )
                .await?;
                stats.updated += 1;
                (row.id, true)
            }
            None => {
                let id = skills_db::insert(
                    db,
                    &skill.name,
                    &skill.description,
                    &skill.body,
                    &trigger_json,
                    &allowed_json,
                    &body_hash,
                    &fs_path_str,
                )
                .await?;
                stats.inserted += 1;
                (id, true)
            }
        };

        if embed_required {
            if let Some(embedder) = embedder.clone() {
                let db = db.clone();
                let body = skill.body.clone();
                tokio::spawn(async move {
                    match embedder.embed(vec![body]).await {
                        Ok(mut vecs) => {
                            if let Some(v) = vecs.pop() {
                                if let Err(e) =
                                    vector::insert(&db, vector::VecTable::Skills, rowid, &v).await
                                {
                                    tracing::warn!(error = %e, rowid, "skill embed vec insert");
                                }
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, rowid, "skill embed"),
                    }
                });
            }
        }
    }

    // Drop skills the user removed from disk.
    let all_rows = skills_db::list_all(db).await?;
    for row in all_rows {
        if !fs_names.contains(&row.name) {
            skills_db::delete_by_name(db, &row.name).await?;
            let _ = vector::delete(db, vector::VecTable::Skills, row.id).await;
            stats.deleted += 1;
        }
    }

    Ok(stats)
}

fn hex_sha256(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let bytes = h.finalize();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ravn_persistence::Db;
    use tempfile::TempDir;

    fn skill(name: &str, body: &str) -> Skill {
        Skill {
            name: name.into(),
            description: format!("desc for {name}"),
            trigger_patterns: vec![],
            allowed_tools: vec![],
            body: body.into(),
            fs_path: std::path::PathBuf::from(format!("/tmp/{name}/SKILL.md")),
        }
    }

    async fn fresh_db() -> (Db, TempDir) {
        let td = TempDir::new().unwrap();
        let db = Db::open(td.path().join("sync.db")).await.unwrap();
        (db, td)
    }

    #[tokio::test]
    async fn inserts_then_marks_unchanged_on_resync() {
        let (db, _td) = fresh_db().await;
        let stats = sync_to_db(&db, vec![skill("a", "body a")], None)
            .await
            .unwrap();
        assert_eq!(stats.inserted, 1);
        assert_eq!(stats.unchanged, 0);

        let stats = sync_to_db(&db, vec![skill("a", "body a")], None)
            .await
            .unwrap();
        assert_eq!(stats.inserted, 0);
        assert_eq!(stats.unchanged, 1);
    }

    #[tokio::test]
    async fn body_change_triggers_update() {
        let (db, _td) = fresh_db().await;
        sync_to_db(&db, vec![skill("a", "body a")], None).await.unwrap();
        let stats = sync_to_db(&db, vec![skill("a", "body a v2")], None)
            .await
            .unwrap();
        assert_eq!(stats.updated, 1);
        assert_eq!(stats.unchanged, 0);

        let row = ravn_persistence::skills::get_by_name(&db, "a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.body, "body a v2");
    }

    #[tokio::test]
    async fn fts_search_finds_inserted_skill() {
        let (db, _td) = fresh_db().await;
        sync_to_db(
            &db,
            vec![
                skill("git-workflow", "How to commit, branch, rebase"),
                skill("note-taking", "Capture ideas in markdown"),
            ],
            None,
        )
        .await
        .unwrap();

        let hits = ravn_persistence::skills::search(&db, "commit", 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "git-workflow");
    }

    #[tokio::test]
    async fn missing_from_fs_gets_deleted() {
        let (db, _td) = fresh_db().await;
        sync_to_db(&db, vec![skill("a", "body a"), skill("b", "body b")], None)
            .await
            .unwrap();
        let stats = sync_to_db(&db, vec![skill("a", "body a")], None)
            .await
            .unwrap();
        assert_eq!(stats.deleted, 1);
        assert!(ravn_persistence::skills::get_by_name(&db, "b")
            .await
            .unwrap()
            .is_none());
    }
}
