//! Persistence-Layer (D2): sqlx default + `rusqlite` in `spawn_blocking` for
//! FTS5/`sqlite-vec` corners (Phase 0 only uses sqlx; rusqlite is in
//! `workspace.dependencies` and will be wired in alongside `sqlite-vec`
//! in Phase 2).
//!
//! Migrations live in `crates/persistence/migrations/` and are embedded at
//! compile time via `sqlx::migrate!`.

pub mod allowlist;
pub mod db;
pub mod error;
pub mod events;
pub mod messages;
pub mod sessions;
pub mod skills;
pub mod vector;

pub use db::{now_millis, Db};
pub use error::Error;
pub use messages::MessageRow;
pub use sessions::{Session, UsageDelta};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn db() -> Db {
        Db::open_in_memory().await.expect("open in-memory db")
    }

    #[tokio::test]
    async fn session_create_and_get() {
        let db = db().await;
        let s = sessions::create(&db, "sess-1", "cli", Some("claude-sonnet-4-6"))
            .await
            .unwrap();
        assert_eq!(s.id, "sess-1");
        assert_eq!(s.channel, "cli");
        assert_eq!(s.input_tokens, 0);
        assert!(s.ended_at.is_none());

        let got = sessions::get(&db, "sess-1").await.unwrap().unwrap();
        assert_eq!(got.model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[tokio::test]
    async fn session_bump_usage_accumulates() {
        let db = db().await;
        sessions::create(&db, "sess-1", "cli", None).await.unwrap();
        sessions::bump_usage(
            &db,
            "sess-1",
            UsageDelta {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 80,
                cache_creation_tokens: 20,
                reasoning_tokens: 10,
                cost_usd: 0.001,
            },
        )
        .await
        .unwrap();
        sessions::bump_usage(
            &db,
            "sess-1",
            UsageDelta {
                input_tokens: 30,
                output_tokens: 20,
                cost_usd: 0.0005,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let s = sessions::get(&db, "sess-1").await.unwrap().unwrap();
        assert_eq!(s.input_tokens, 130);
        assert_eq!(s.output_tokens, 70);
        assert_eq!(s.cache_read_tokens, 80);
        assert_eq!(s.reasoning_tokens, 10);
        assert!((s.cost_usd - 0.0015).abs() < 1e-9);
    }

    #[tokio::test]
    async fn message_append_and_list() {
        let db = db().await;
        sessions::create(&db, "sess-1", "cli", None).await.unwrap();

        let content = serde_json::to_string(&json!([
            {"type": "text", "text": "hello"}
        ]))
        .unwrap();
        let id1 = messages::append(&db, "sess-1", "user", &content)
            .await
            .unwrap();
        let id2 = messages::append(&db, "sess-1", "assistant", &content)
            .await
            .unwrap();
        assert!(id2 > id1);

        let rows = messages::list_session(&db, "sess-1").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].role, "user");
        assert_eq!(rows[1].role, "assistant");
    }

    #[tokio::test]
    async fn fts_search_finds_message() {
        let db = db().await;
        sessions::create(&db, "sess-1", "cli", None).await.unwrap();
        messages::append(&db, "sess-1", "user", "weather forecast for berlin")
            .await
            .unwrap();
        messages::append(&db, "sess-1", "user", "what is rust ownership")
            .await
            .unwrap();

        let hits = messages::search(&db, "berlin", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("berlin"));

        let hits = messages::search(&db, "rust", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    fn unit_vec(seed: f32) -> Vec<f32> {
        let raw: Vec<f32> = (0..vector::EMBEDDING_DIM)
            .map(|i| (i as f32 * 0.001) + seed)
            .collect();
        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        raw.into_iter().map(|x| x / norm).collect()
    }

    #[tokio::test]
    async fn hybrid_search_falls_back_to_fts_with_empty_vec() {
        let db = db().await;
        sessions::create(&db, "s", "cli", None).await.unwrap();
        messages::append(&db, "s", "user", "berlin weather").await.unwrap();
        let hits = messages::search_hybrid(&db, "berlin", &[], 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("berlin"));
    }

    #[tokio::test]
    async fn hybrid_search_merges_fts_and_vec_via_rrf() {
        // Tempfile DB so vec0 (rusqlite) and FTS5 (sqlx) share storage.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("hybrid.db");
        let db = Db::open(&path).await.unwrap();
        sessions::create(&db, "s", "cli", None).await.unwrap();

        // Three messages — only one mentions "rust"; embeddings are
        // contrived such that vec ranks message id=3 first.
        let m1 = messages::append(&db, "s", "user", "berlin weather").await.unwrap();
        let m2 = messages::append(&db, "s", "user", "tokyo rust meetup").await.unwrap();
        let m3 = messages::append(&db, "s", "user", "paris cafe notes").await.unwrap();

        vector::insert(&db, vector::VecTable::Messages, m1, &unit_vec(2.0))
            .await
            .unwrap();
        vector::insert(&db, vector::VecTable::Messages, m2, &unit_vec(1.0))
            .await
            .unwrap();
        vector::insert(&db, vector::VecTable::Messages, m3, &unit_vec(0.0))
            .await
            .unwrap();

        // Hybrid query: text "rust" only matches m2, vec query close to m3.
        // RRF should rank m2 (text rank 1) ahead of m3 (vec rank 1) since
        // they tie on RRF score and the merge keeps both, but m2 has 1
        // contribution out of FTS5 and m3 has 1 out of vec — they tie.
        // What we actually assert: the union of m2 + m3 appears in the
        // top 2.
        let hits = messages::search_hybrid(&db, "rust", &unit_vec(0.0), 2)
            .await
            .unwrap();
        let ids: std::collections::HashSet<i64> = hits.iter().map(|r| r.id).collect();
        assert!(ids.contains(&m2));
        assert!(ids.contains(&m3));
    }

    #[tokio::test]
    async fn get_by_ids_preserves_order() {
        let db = db().await;
        sessions::create(&db, "s", "cli", None).await.unwrap();
        let a = messages::append(&db, "s", "user", "a").await.unwrap();
        let b = messages::append(&db, "s", "user", "b").await.unwrap();
        let c = messages::append(&db, "s", "user", "c").await.unwrap();
        let rows = messages::get_by_ids(&db, &[c, a, b]).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![c, a, b]);
    }

    #[tokio::test]
    async fn allowlist_insert_then_list() {
        let db = db().await;
        allowlist::insert(&db, "shell").await.unwrap();
        allowlist::insert(&db, "file_write").await.unwrap();
        let names = allowlist::list_all(&db).await.unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"shell".to_string()));
        assert!(allowlist::contains(&db, "shell").await.unwrap());
        assert!(!allowlist::contains(&db, "datetime").await.unwrap());
    }

    #[tokio::test]
    async fn allowlist_insert_idempotent_then_remove() {
        let db = db().await;
        allowlist::insert(&db, "shell").await.unwrap();
        allowlist::insert(&db, "shell").await.unwrap();
        assert_eq!(allowlist::list_all(&db).await.unwrap().len(), 1);
        allowlist::remove(&db, "shell").await.unwrap();
        assert!(!allowlist::contains(&db, "shell").await.unwrap());
    }

    #[tokio::test]
    async fn event_append_and_count() {
        let db = db().await;
        sessions::create(&db, "sess-1", "cli", None).await.unwrap();
        let payload = json!({"step": 1, "thought": "decide"});
        events::append_json(&db, Some("trace-1"), Some("sess-1"), "react.step", &payload)
            .await
            .unwrap();
        events::append_json(&db, Some("trace-1"), Some("sess-1"), "react.step", &payload)
            .await
            .unwrap();
        assert_eq!(events::count(&db).await.unwrap(), 2);
    }
}
