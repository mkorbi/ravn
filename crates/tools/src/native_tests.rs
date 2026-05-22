//! Unit tests for the seven Phase-1 native tools.

use std::sync::Arc;

use ravn_persistence::Db;
use serde_json::json;
use tempfile::TempDir;

use crate::native::{
    register_defaults, DateTime, FileRead, FileWrite, MemorySave, SessionSearch, Shell,
};

fn no_embedder() -> Option<std::sync::Arc<ravn_embeddings::Embedder>> {
    None
}
use crate::{AllowAll, Permission, Tool, ToolContext, ToolError, ToolRegistry};

async fn ctx() -> (ToolContext, Db) {
    let db = Db::open_in_memory().await.unwrap();
    ravn_persistence::sessions::create(&db, "sess", "test", None)
        .await
        .unwrap();
    let c = ToolContext::new(db.clone(), "sess", "trace", Arc::new(AllowAll));
    (c, db)
}

// --- file_read ---------------------------------------------------------

#[tokio::test]
async fn file_read_returns_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hi.txt");
    tokio::fs::write(&path, "hello world").await.unwrap();
    let (c, _) = ctx().await;
    let out = FileRead
        .invoke(json!({"path": path.to_str().unwrap()}), &c)
        .await
        .unwrap();
    assert_eq!(out.content, "hello world");
    assert!(out.trustworthy);
}

#[tokio::test]
async fn file_read_rejects_directory() {
    let dir = TempDir::new().unwrap();
    let (c, _) = ctx().await;
    let err = FileRead
        .invoke(json!({"path": dir.path().to_str().unwrap()}), &c)
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[tokio::test]
async fn file_read_truncates_at_max_bytes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("big.txt");
    tokio::fs::write(&path, "x".repeat(10_000)).await.unwrap();
    let (c, _) = ctx().await;
    let out = FileRead
        .invoke(
            json!({"path": path.to_str().unwrap(), "max_bytes": 100}),
            &c,
        )
        .await
        .unwrap();
    assert!(out.content.contains("[truncated"));
}

// --- file_write --------------------------------------------------------

#[tokio::test]
async fn file_write_round_trips() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.txt");
    let (c, _) = ctx().await;
    FileWrite
        .invoke(
            json!({"path": path.to_str().unwrap(), "content": "rust"}),
            &c,
        )
        .await
        .unwrap();
    let read_back = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(read_back, "rust");
}

#[tokio::test]
async fn file_write_create_dirs_makes_parents() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("a/b/c/out.txt");
    let (c, _) = ctx().await;
    FileWrite
        .invoke(
            json!({
                "path": nested.to_str().unwrap(),
                "content": "ok",
                "create_dirs": true,
            }),
            &c,
        )
        .await
        .unwrap();
    assert!(nested.exists());
}

// --- shell -------------------------------------------------------------

#[tokio::test]
async fn shell_echo_succeeds() {
    let (c, _) = ctx().await;
    let out = Shell
        .invoke(json!({"command": "printf hello"}), &c)
        .await
        .unwrap();
    assert!(out.content.contains("hello"));
    assert!(out.content.contains("exit=0"));
    assert!(!out.is_error);
}

#[tokio::test]
async fn shell_nonzero_exit_marked_error() {
    let (c, _) = ctx().await;
    let out = Shell
        .invoke(json!({"command": "exit 7"}), &c)
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(out.content.contains("exit=7"));
}

#[tokio::test]
async fn shell_timeout_fires() {
    let (c, _) = ctx().await;
    let err = Shell
        .invoke(
            json!({"command": "sleep 5", "timeout_secs": 1}),
            &c,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Io(s) if s.contains("timeout")));
}

// --- datetime ----------------------------------------------------------

#[tokio::test]
async fn datetime_returns_iso_default() {
    let (c, _) = ctx().await;
    let out = DateTime.invoke(json!({}), &c).await.unwrap();
    // RFC 3339 contains a `T` between date and time and ends with offset.
    assert!(out.content.contains('T'));
    assert!(!out.content.is_empty());
}

#[tokio::test]
async fn datetime_respects_format() {
    let (c, _) = ctx().await;
    let out = DateTime
        .invoke(json!({"format": "%Y", "timezone": "utc"}), &c)
        .await
        .unwrap();
    assert_eq!(out.content.len(), 4);
    assert!(out.content.chars().all(|ch| ch.is_ascii_digit()));
}

// --- session_search ----------------------------------------------------

#[tokio::test]
async fn session_search_finds_message() {
    let (c, db) = ctx().await;
    ravn_persistence::messages::append(&db, "sess", "user", "berlin weather forecast")
        .await
        .unwrap();
    let out = SessionSearch::new(no_embedder())
        .invoke(json!({"query": "berlin"}), &c)
        .await
        .unwrap();
    assert!(out.content.contains("berlin"));
    assert!(out.content.contains("1 hit"));
}

#[tokio::test]
async fn session_search_empty_returns_no_hits_message() {
    let (c, _) = ctx().await;
    let out = SessionSearch::new(no_embedder())
        .invoke(json!({"query": "zzznothing"}), &c)
        .await
        .unwrap();
    assert!(out.content.starts_with("no hits"));
}

// --- memory_save -------------------------------------------------------

#[tokio::test]
async fn memory_save_appends_section() {
    let dir = TempDir::new().unwrap();
    let (c, _) = ctx().await;
    let tool = MemorySave {
        data_dir: dir.path().to_path_buf(),
    };
    tool.invoke(
        json!({"slot": "memory", "body": "x prefers rust", "section": "2026-05-15"}),
        &c,
    )
    .await
    .unwrap();
    let content = tokio::fs::read_to_string(dir.path().join("memory.md"))
        .await
        .unwrap();
    assert!(content.contains("## 2026-05-15"));
    assert!(content.contains("x prefers rust"));
}

#[tokio::test]
async fn memory_save_replace_overwrites() {
    let dir = TempDir::new().unwrap();
    let (c, _) = ctx().await;
    let tool = MemorySave {
        data_dir: dir.path().to_path_buf(),
    };
    tool.invoke(json!({"slot": "soul", "body": "first"}), &c)
        .await
        .unwrap();
    tool.invoke(
        json!({"slot": "soul", "body": "second", "mode": "replace"}),
        &c,
    )
    .await
    .unwrap();
    let content = tokio::fs::read_to_string(dir.path().join("soul.md"))
        .await
        .unwrap();
    assert_eq!(content, "second");
}

// --- registry / schemas -----------------------------------------------

#[tokio::test]
async fn register_defaults_registers_all() {
    let mut reg = ToolRegistry::new();
    let dir = TempDir::new().unwrap();
    register_defaults(&mut reg, dir.path().to_path_buf(), no_embedder());
    assert_eq!(reg.len(), 10);

    let names: std::collections::HashSet<_> = reg.names().collect();
    for expected in [
        "file_read",
        "file_write",
        "shell",
        "web_fetch",
        "session_search",
        "memory_save",
        "world_write",
        "datetime",
        "skill_list",
        "skill_view",
    ] {
        assert!(names.contains(expected), "missing tool: {expected}");
    }
}

#[tokio::test]
async fn permissions_match_phase1_spec() {
    let mut reg = ToolRegistry::new();
    let dir = TempDir::new().unwrap();
    register_defaults(&mut reg, dir.path().to_path_buf(), no_embedder());

    let perm = |name: &str| reg.get(name).unwrap().permission();
    assert_eq!(perm("file_read"), Permission::Read);
    assert_eq!(perm("web_fetch"), Permission::Read);
    assert_eq!(perm("session_search"), Permission::Read);
    assert_eq!(perm("datetime"), Permission::Read);
    assert_eq!(perm("skill_list"), Permission::Read);
    assert_eq!(perm("skill_view"), Permission::Read);
    assert_eq!(perm("file_write"), Permission::Write);
    assert_eq!(perm("memory_save"), Permission::Write);
    assert_eq!(perm("world_write"), Permission::Write);
    assert_eq!(perm("shell"), Permission::Exec);
}

#[tokio::test]
async fn read_only_subset_drops_write_exec_and_excluded_names() {
    let mut reg = ToolRegistry::new();
    let dir = TempDir::new().unwrap();
    register_defaults(&mut reg, dir.path().to_path_buf(), no_embedder());
    // Pretend subagent_delegate is registered too (we test exclude
    // semantics on a real Read-tool here).
    let sub = reg.read_only_subset(&["session_search"]);
    let names: std::collections::HashSet<_> = sub.names().collect();
    // Read tools that survive (datetime/file_read/web_fetch/skill_*)
    assert!(names.contains("file_read"));
    assert!(names.contains("datetime"));
    assert!(names.contains("skill_list"));
    // Excluded by name.
    assert!(!names.contains("session_search"));
    // Excluded by permission.
    assert!(!names.contains("file_write"));
    assert!(!names.contains("shell"));
    assert!(!names.contains("memory_save"));
    assert!(!names.contains("world_write"));
}

#[tokio::test]
async fn as_schemas_produces_jsonschema_per_tool() {
    let mut reg = ToolRegistry::new();
    let dir = TempDir::new().unwrap();
    register_defaults(&mut reg, dir.path().to_path_buf(), no_embedder());

    let schemas = reg.as_schemas();
    assert_eq!(schemas.len(), 10);
    for s in &schemas {
        assert!(!s.name.is_empty());
        assert!(!s.description.is_empty());
        // Each schema must be a JSON object with `properties`.
        assert!(s.parameters.is_object(), "{}: not an object", s.name);
        let obj = s.parameters.as_object().unwrap();
        assert!(
            obj.contains_key("properties") || obj.contains_key("$ref") || obj.contains_key("type"),
            "{}: schema missing properties/type/$ref",
            s.name
        );
    }
    // Sorted alphabetically.
    let names: Vec<_> = schemas.iter().map(|s| s.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}
