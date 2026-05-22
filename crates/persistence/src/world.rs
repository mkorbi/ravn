//! Persistent world state (Phase 4.11).
//!
//! A single typed [`WorldState`] — the agent's durable model of the user's
//! world (active projects, open browser tabs, things to watch) — serialized as
//! JSON into a one-row table. It is loaded into every prompt by
//! `ravn_core::Agent::run` (so the agent is always aware of it, like
//! `MEMORY.md`) and mutated through the `world_write` tool. Heartbeat jobs
//! (Phase 4.10) act on the `watch_targets`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::{now_millis, Db};
use crate::error::Error;

/// The agent's durable model of the user's world. Serialized whole into the
/// single `world_state` row; replaced atomically on each write.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorldState {
    /// Projects the user is actively working on.
    #[serde(default)]
    pub projects: Vec<Project>,
    /// Browser tabs / documents the user is keeping open.
    #[serde(default)]
    pub open_tabs: Vec<OpenTab>,
    /// Things the agent should keep an eye on — heartbeat jobs read these.
    #[serde(default)]
    pub watch_targets: Vec<WatchTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    pub name: String,
    /// Free-form status, e.g. "active", "paused", "done".
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenTab {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WatchTarget {
    /// Human label, e.g. "PR #42 CI status".
    pub label: String,
    /// What to check — a URL, file path, or search query.
    pub query: String,
    /// Unix-millis of the last time a heartbeat checked this, if ever.
    #[serde(default)]
    pub last_checked: Option<i64>,
}

impl WorldState {
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.open_tabs.is_empty() && self.watch_targets.is_empty()
    }

    /// Render as a compact Markdown block for prompt injection. Returns an
    /// empty string when there is nothing to show (so the caller can skip
    /// injection and keep the prompt prefix stable).
    pub fn render_markdown(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::from("# World State");
        if !self.projects.is_empty() {
            out.push_str("\n\n## Projects\n");
            for p in &self.projects {
                let status = if p.status.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", p.status)
                };
                let notes = if p.notes.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", p.notes)
                };
                out.push_str(&format!("- {}{}{}\n", p.name, status, notes));
            }
        }
        if !self.open_tabs.is_empty() {
            out.push_str("\n## Open tabs\n");
            for t in &self.open_tabs {
                out.push_str(&format!("- {} ({})\n", t.title, t.url));
            }
        }
        if !self.watch_targets.is_empty() {
            out.push_str("\n## Watch targets\n");
            for w in &self.watch_targets {
                out.push_str(&format!("- {}: {}\n", w.label, w.query));
            }
        }
        out
    }
}

/// Load the world state. Returns [`WorldState::default`] when no row exists.
pub async fn load(db: &Db) -> Result<WorldState, Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT data FROM world_state WHERE id = 1")
        .fetch_optional(&db.pool)
        .await?;
    match row {
        Some((json,)) => Ok(serde_json::from_str(&json)?),
        None => Ok(WorldState::default()),
    }
}

/// Persist the world state, replacing the single row (`id = 1`).
pub async fn save(db: &Db, state: &WorldState) -> Result<(), Error> {
    let json = serde_json::to_string(state)?;
    sqlx::query(
        "INSERT INTO world_state (id, data, updated_at) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
    )
    .bind(json)
    .bind(now_millis())
    .execute(&db.pool)
    .await?;
    Ok(())
}
