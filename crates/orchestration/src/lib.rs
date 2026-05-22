//! State-machine orchestration with persistent checkpoints
//! (Phase 3.6 + 3.7).
//!
//! Why a state graph alongside the ReAct loop? ReAct is fine for "ask
//! → answer with tools" turns; complex workflows (multi-step plans,
//! LATS search, custom pipelines) want explicit nodes + edges + the
//! ability to resume after a crash. This module provides the minimal
//! typed state graph from `project.md` §1.7 and a postcard-backed
//! checkpoint store in the existing `events` table.
//!
//! Phase 3 ships only the primitives. Phase 4+ will use them for
//! Plan-and-Execute (subagents) and Phase 6 for LATS.

pub mod checkpoint;
pub mod graph;

#[cfg(test)]
mod tests;

pub use checkpoint::{
    load_typed, save_typed, Checkpoint, CheckpointStore, DbCheckpointStore, MemoryCheckpointStore,
};
pub use graph::{GraphContext, Node, NodeId, StateGraph, END};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown node: {0}")]
    UnknownNode(String),

    #[error("node {node} failed: {reason}")]
    NodeFailed { node: String, reason: String },

    #[error("cancelled")]
    Cancelled,

    #[error("postcard encode: {0}")]
    Encode(String),

    #[error("postcard decode: {0}")]
    Decode(String),

    #[error("persistence: {0}")]
    Persistence(#[from] ravn_persistence::Error),

    #[error("no checkpoint found for trace_id {0}")]
    NoCheckpoint(String),
}
