-- Phase 0.6 initial schema.
-- Notes:
--   * `messages.content` stores a JSON-serialized Vec<ContentBlock>
--     so the FTS5 index over it indexes the JSON; in Phase 1 we will
--     replace this with a flattened text column for clean tokenization.
--   * `events.payload` is BLOB; we encode as UTF-8 JSON for Phase 0,
--     migrating to postcard binary in Phase 1 without a schema change.
--   * `messages_vec` (sqlite-vec vec0 virtual table) is added in
--     Phase 2 — not part of this migration.

CREATE TABLE sessions (
    id                      TEXT    PRIMARY KEY,
    started_at              INTEGER NOT NULL,
    ended_at                INTEGER,
    channel                 TEXT    NOT NULL,
    model                   TEXT,
    input_tokens            INTEGER NOT NULL DEFAULT 0,
    output_tokens           INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens       INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens   INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens        INTEGER NOT NULL DEFAULT 0,
    cost_usd                REAL    NOT NULL DEFAULT 0
);

CREATE INDEX idx_sessions_started_at ON sessions(started_at DESC);

CREATE TABLE messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role        TEXT    NOT NULL,
    content     TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX idx_messages_session_id ON messages(session_id, created_at);

-- Full-text search mirror over messages.content.
CREATE VIRTUAL TABLE messages_fts USING fts5(
    content,
    content='messages',
    content_rowid='id'
);

CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
END;
CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

-- Append-only event log for tracing, trajectories, and audit.
CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id    TEXT,
    session_id  TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    kind        TEXT    NOT NULL,
    payload     BLOB    NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX idx_events_trace   ON events(trace_id, created_at);
CREATE INDEX idx_events_session ON events(session_id, created_at);
CREATE INDEX idx_events_kind    ON events(kind, created_at);
