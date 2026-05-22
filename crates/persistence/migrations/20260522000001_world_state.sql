-- Phase 4.11 — persistent world state.
-- The agent's durable model of the user's world (active projects, open tabs,
-- watch targets) is a single typed struct serialized as JSON into one row.
CREATE TABLE world_state (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    data       TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
);
