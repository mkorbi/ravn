//! Checkpoint store (Phase 3.7).
//!
//! Persists postcard-encoded graph state + next-node-id via the
//! `events` table (`kind = "graph.checkpoint"`). `load_raw` returns
//! the latest row for a given `trace_id`. Last-write-wins by
//! `created_at`.
//!
//! The store trait is object-safe (operates on `Vec<u8>`); typed
//! [`save_typed`] / [`load_typed`] wrap it with postcard so callers
//! don't see raw bytes.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ravn_persistence::Db;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::graph::NodeId;
use crate::Error;

/// Typed snapshot returned to graph callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint<S> {
    pub next_node: &'static str,
    pub state: S,
}

/// On-the-wire form. `&'static str` can't serialize, so the node id
/// round-trips as `String`; on load we leak it back to `&'static str`.
/// Leak is bounded by the number of distinct node ids per process.
#[derive(Serialize, Deserialize)]
struct WireCheckpoint<S> {
    next_node: String,
    state: S,
}

/// Object-safe persistence trait — methods operate on `Vec<u8>` so
/// `Arc<dyn CheckpointStore>` is valid. The typed `save_typed` /
/// `load_typed` helpers wrap it.
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn save_raw(
        &self,
        trace_id: &str,
        session_id: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<(), Error>;

    async fn load_raw(&self, trace_id: &str) -> Result<Vec<u8>, Error>;
}

pub async fn save_typed<S: Serialize + Send + Sync>(
    store: &dyn CheckpointStore,
    trace_id: &str,
    session_id: Option<&str>,
    checkpoint: &Checkpoint<S>,
) -> Result<(), Error> {
    let wire = WireCheckpoint {
        next_node: checkpoint.next_node.to_string(),
        state: &checkpoint.state,
    };
    let bytes = postcard::to_allocvec(&wire).map_err(|e| Error::Encode(e.to_string()))?;
    store.save_raw(trace_id, session_id, bytes).await
}

pub async fn load_typed<S: DeserializeOwned + Send + Sync>(
    store: &dyn CheckpointStore,
    trace_id: &str,
) -> Result<Checkpoint<S>, Error> {
    let bytes = store.load_raw(trace_id).await?;
    let wire: WireCheckpoint<S> =
        postcard::from_bytes(&bytes).map_err(|e| Error::Decode(e.to_string()))?;
    let next_node: NodeId = Box::leak(wire.next_node.into_boxed_str());
    Ok(Checkpoint {
        next_node,
        state: wire.state,
    })
}

pub struct DbCheckpointStore {
    db: Db,
}

impl DbCheckpointStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl CheckpointStore for DbCheckpointStore {
    async fn save_raw(
        &self,
        trace_id: &str,
        session_id: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        ravn_persistence::events::append(
            &self.db,
            Some(trace_id),
            session_id,
            "graph.checkpoint",
            &bytes,
        )
        .await?;
        Ok(())
    }

    async fn load_raw(&self, trace_id: &str) -> Result<Vec<u8>, Error> {
        ravn_persistence::events::latest_payload(&self.db, trace_id, "graph.checkpoint")
            .await?
            .ok_or_else(|| Error::NoCheckpoint(trace_id.to_string()))
    }
}

/// In-memory store — tests and short-lived workflows.
pub struct MemoryCheckpointStore {
    inner: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryCheckpointStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemoryCheckpointStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CheckpointStore for MemoryCheckpointStore {
    async fn save_raw(
        &self,
        trace_id: &str,
        _session_id: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        self.inner
            .lock()
            .unwrap()
            .insert(trace_id.to_string(), bytes);
        Ok(())
    }

    async fn load_raw(&self, trace_id: &str) -> Result<Vec<u8>, Error> {
        self.inner
            .lock()
            .unwrap()
            .get(trace_id)
            .cloned()
            .ok_or_else(|| Error::NoCheckpoint(trace_id.to_string()))
    }
}
