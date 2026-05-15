-- Phase 2.4 / D11: Hybrid skills storage — filesystem at
-- ~/.ravn/skills/<name>/SKILL.md is canonical; this DB mirror powers
-- FTS5 search (and the sqlite-vec skills_vec virtual table, which is
-- created in vector::bootstrap with matching rowid keying).

CREATE TABLE skills (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    name              TEXT    UNIQUE NOT NULL,
    description       TEXT    NOT NULL,
    body              TEXT    NOT NULL,
    -- JSON-encoded Vec<String> — kept as text since FTS5 / vec ops don't
    -- need to query into them.
    trigger_patterns  TEXT    NOT NULL DEFAULT '[]',
    allowed_tools     TEXT    NOT NULL DEFAULT '[]',
    -- SHA-256 hex of body, used to skip re-embedding unchanged skills.
    body_hash         TEXT    NOT NULL,
    fs_path           TEXT    NOT NULL,
    indexed_at        INTEGER NOT NULL
);

CREATE INDEX idx_skills_name ON skills(name);

CREATE VIRTUAL TABLE skills_fts USING fts5(
    name, description, body,
    content='skills',
    content_rowid='id'
);

CREATE TRIGGER skills_ai AFTER INSERT ON skills BEGIN
    INSERT INTO skills_fts(rowid, name, description, body)
    VALUES (new.id, new.name, new.description, new.body);
END;
CREATE TRIGGER skills_ad AFTER DELETE ON skills BEGIN
    INSERT INTO skills_fts(skills_fts, rowid, name, description, body)
    VALUES('delete', old.id, old.name, old.description, old.body);
END;
CREATE TRIGGER skills_au AFTER UPDATE ON skills BEGIN
    INSERT INTO skills_fts(skills_fts, rowid, name, description, body)
    VALUES('delete', old.id, old.name, old.description, old.body);
    INSERT INTO skills_fts(rowid, name, description, body)
    VALUES (new.id, new.name, new.description, new.body);
END;
