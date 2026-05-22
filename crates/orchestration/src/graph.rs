//! Typed state graph (Phase 3.6).
//!
//! A `StateGraph<S>` is a directed graph of `Node<S>` impls. Each node
//! mutates the shared state `S` and returns the [`NodeId`] of the
//! successor to execute. `END` terminates the run.
//!
//! The graph is created via [`StateGraph::new`] + [`StateGraph::add_node`]
//! and run via [`StateGraph::run`]. After each node's transition the
//! state is snapshotted via the configured [`CheckpointStore`]
//! (Phase 3.7); a crash mid-run can be resumed via
//! [`StateGraph::resume`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ravn_persistence::Db;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::checkpoint::{
    load_typed, save_typed, Checkpoint, CheckpointStore, DbCheckpointStore,
};
use crate::Error;

/// Stable identifier for a graph node. Always `&'static str` so the
/// graph topology lives at compile time (a runtime-built graph would
/// need a different ID type).
pub type NodeId = &'static str;

/// Reserved sentinel meaning "stop the graph run". Returning this from
/// a [`Node::run`] terminates [`StateGraph::run`] gracefully.
pub const END: NodeId = "__end__";

/// Per-run context handed to every [`Node::run`]. Holds the DB handle
/// for state persistence and a `CancellationToken` so long-running
/// nodes can shut down promptly.
#[derive(Clone)]
pub struct GraphContext {
    pub db: Db,
    pub trace_id: String,
    pub session_id: Option<String>,
    pub cancel: CancellationToken,
}

impl GraphContext {
    pub fn new(db: Db, trace_id: impl Into<String>) -> Self {
        Self {
            db,
            trace_id: trace_id.into(),
            session_id: None,
            cancel: CancellationToken::new(),
        }
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }
}

#[async_trait]
pub trait Node<S>: Send + Sync {
    /// Run this node. Mutates `state` in place; returns the
    /// [`NodeId`] of the successor to execute. Return [`END`] to
    /// terminate the run.
    async fn run(&self, state: &mut S, ctx: &GraphContext) -> Result<NodeId, Error>;
}

pub struct StateGraph<S> {
    nodes: HashMap<NodeId, Arc<dyn Node<S>>>,
    entry: NodeId,
    store: Arc<dyn CheckpointStore>,
}

impl<S> StateGraph<S>
where
    S: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    pub fn new(entry: NodeId, db: Db) -> Self {
        Self {
            nodes: HashMap::new(),
            entry,
            store: Arc::new(DbCheckpointStore::new(db)),
        }
    }

    /// Replace the default DB-backed [`CheckpointStore`] (e.g. with an
    /// in-memory one for tests).
    pub fn with_store(mut self, store: Arc<dyn CheckpointStore>) -> Self {
        self.store = store;
        self
    }

    pub fn add_node<N: Node<S> + 'static>(mut self, id: NodeId, node: N) -> Self {
        self.nodes.insert(id, Arc::new(node));
        self
    }

    /// Drive the graph from `entry` to `END`. Snapshots `state` and
    /// the current node id after every node transition.
    pub async fn run(&self, initial: S, ctx: &GraphContext) -> Result<S, Error> {
        let mut state = initial;
        let mut current = self.entry;
        loop {
            if ctx.cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if current == END {
                self.snapshot(&state, END, ctx).await?;
                return Ok(state);
            }
            let node = self
                .nodes
                .get(current)
                .ok_or_else(|| Error::UnknownNode(current.to_string()))?
                .clone();
            let next = node.run(&mut state, ctx).await?;
            self.snapshot(&state, next, ctx).await?;
            current = next;
        }
    }

    /// Continue from the latest checkpoint for `ctx.trace_id`. If the
    /// last snapshot was at `END`, returns the persisted state without
    /// running anything; otherwise resumes from the recorded next-node.
    pub async fn resume(&self, ctx: &GraphContext) -> Result<S, Error> {
        let cp: Checkpoint<S> = load_typed::<S>(&*self.store, &ctx.trace_id).await?;
        if cp.next_node == END {
            return Ok(cp.state);
        }
        let mut state = cp.state;
        let mut current = cp.next_node;
        loop {
            if ctx.cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if current == END {
                self.snapshot(&state, END, ctx).await?;
                return Ok(state);
            }
            let node = self
                .nodes
                .get(current)
                .ok_or_else(|| Error::UnknownNode(current.to_string()))?
                .clone();
            let next = node.run(&mut state, ctx).await?;
            self.snapshot(&state, next, ctx).await?;
            current = next;
        }
    }

    async fn snapshot(&self, state: &S, next_node: NodeId, ctx: &GraphContext) -> Result<(), Error> {
        let cp = Checkpoint {
            next_node,
            state: state.clone(),
        };
        save_typed(&*self.store, &ctx.trace_id, ctx.session_id.as_deref(), &cp).await
    }
}
