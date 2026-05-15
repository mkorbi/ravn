//! Hybrid skill storage — filesystem canonical + DB mirror (Phase 2.4, D11).
//!
//! Skills live on disk at `~/.ravn/skills/<name>/SKILL.md`:
//!
//! ```markdown
//! ---
//! name: git-workflow
//! description: |
//!   Use when the user wants to commit, branch, rebase, manage Git.
//! trigger_patterns: ["commit", "git status", "merge conflict"]
//! allowed_tools: [shell, file_read, file_write]
//! ---
//! # Git Workflow Skill
//!
//! ## When to use
//! ...
//! ```
//!
//! [`load_all_from_fs`] scans a directory, parses each `SKILL.md`, and
//! returns `Vec<Skill>`. [`sync_to_db`] upserts them into the DB
//! mirror, computing a SHA-256 body hash to skip re-embedding
//! unchanged skills.
//!
//! For new or changed bodies, a `tokio::spawn` fires off an embedding
//! call and writes to `skills_vec` (Phase 2.5 will use the vec index
//! for top-K matching).

pub mod parser;
pub mod sync;

pub use parser::{load_all_from_fs, load_skill, Skill};
pub use sync::sync_to_db;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(String),
    #[error("parse SKILL.md at {path}: {reason}")]
    Parse { path: String, reason: String },
    #[error("persistence: {0}")]
    Persistence(#[from] ravn_persistence::Error),
}
