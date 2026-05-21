---
title: Persistence
description: SQLite, FTS5, sqlite-vec, hybrid search.
sidebar:
  order: 3
---

ravn keeps everything in one SQLite database at `~/.ravn/state.db`
(WAL-mode). Two access paths share it:

- **`sqlx`** for normal CRUD on `sessions`, `messages`, `events`,
  `skills`, `tool_allowlist`. Compile-time SQL checking, async.
- **`rusqlite`** in `tokio::spawn_blocking` for `sqlite-vec`
  operations. The vec extension is loaded once per process via
  `sqlite3_auto_extension`.

The two libraries never share a connection — but they share the
database file. WAL-mode means concurrent readers + a single writer
across either library work fine.

## Schema

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    started_at INTEGER,
    ended_at INTEGER,
    channel TEXT,
    model TEXT,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    cache_creation_tokens INTEGER DEFAULT 0,
    reasoning_tokens INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT, content TEXT, created_at INTEGER
);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    content, content='messages', content_rowid='id'
);

-- Embedding index (sqlite-vec, 768 dim, EmbeddingGemma-300M).
-- Created out-of-band by ravn_persistence::vector::bootstrap;
-- not part of the sqlx migration set.
CREATE VIRTUAL TABLE messages_vec USING vec0(embedding float[768]);
CREATE VIRTUAL TABLE skills_vec   USING vec0(embedding float[768]);

CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id TEXT, session_id TEXT,
    kind TEXT, payload BLOB, created_at INTEGER
);

CREATE TABLE skills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE, description TEXT, body TEXT,
    trigger_patterns TEXT, allowed_tools TEXT,
    body_hash TEXT, fs_path TEXT, indexed_at INTEGER
);

CREATE TABLE tool_allowlist (
    tool_name TEXT PRIMARY KEY,
    created_at INTEGER
);
```

## Hybrid search

`ravn_persistence::messages::search_hybrid` runs FTS5 and the vec0
k-NN in parallel via `tokio::join`, then merges with Reciprocal Rank
Fusion (k = 60, 2× overfetch):

```
score(doc) = Σ_source 1 / (k_rrf + rank_source(doc))
```

The result is robust against either index returning surprising
ordering — keyword-only and semantic-only rankings each contribute,
and ties broken by doc-id mean the merge is deterministic.

The `session_search` tool calls `search_hybrid` when an `Embedder` is
configured (default), otherwise falls back to FTS5-only.

## Embeddings pipeline

When `Agent::run` persists a message, it also fire-and-forgets:

```rust
tokio::spawn(async move {
    let vec = embedder.embed(vec![text]).await?;
    vector::insert(&db, VecTable::Messages, rowid, &vec).await?;
});
```

Failures `tracing::warn` but don't block the agent loop. The same
pattern handles skill bodies on sync (with a SHA-256 body-hash gate
so unchanged skills don't re-embed).

## sqlite-vec quirks

Two things to know:

1. The standalone `sqlite3` CLI **can't** read `vec0` tables —
   sqlite-vec is statically linked against rusqlite, there's no
   loadable `.dylib`. Use ravn's own helpers, or write a small Rust
   program that loads the extension via `rusqlite::Connection`.

2. The embedding dim is baked into the `CREATE VIRTUAL TABLE` SQL.
   `vector::bootstrap` reads `sqlite_schema.sql` to detect dim
   mismatch (e.g. if you switch embedding model) and drops + recreates
   the table at the new dim. That throws away the old embeddings —
   they're useless against the new model anyway.

## Locating your DB

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/ravn/state.db` |
| Linux | `${XDG_DATA_HOME:-$HOME/.local/share}/ravn/state.db` |

Inspect with `sqlite3 …/state.db ".tables"` for the regular ones;
`vec0` queries need a Rust helper.
