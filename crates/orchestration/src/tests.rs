//! Integration tests for StateGraph + Checkpoint round-trips.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::checkpoint::{load_typed, save_typed, Checkpoint, MemoryCheckpointStore};
use crate::graph::{GraphContext, Node, NodeId, StateGraph, END};
use crate::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Counter {
    value: i64,
    history: Vec<String>,
}

struct Increment;

#[async_trait]
impl Node<Counter> for Increment {
    async fn run(&self, state: &mut Counter, _ctx: &GraphContext) -> Result<NodeId, Error> {
        state.value += 1;
        state.history.push("increment".into());
        if state.value >= 3 {
            Ok("finalize")
        } else {
            Ok("increment")
        }
    }
}

struct Finalize;

#[async_trait]
impl Node<Counter> for Finalize {
    async fn run(&self, state: &mut Counter, _ctx: &GraphContext) -> Result<NodeId, Error> {
        state.history.push("finalize".into());
        Ok(END)
    }
}

async fn graph() -> (StateGraph<Counter>, GraphContext) {
    let db = ravn_persistence::Db::open_in_memory().await.unwrap();
    let graph = StateGraph::<Counter>::new("increment", db.clone())
        .with_store(Arc::new(MemoryCheckpointStore::new()))
        .add_node("increment", Increment)
        .add_node("finalize", Finalize);
    let ctx = GraphContext::new(db, "trace-1");
    (graph, ctx)
}

#[tokio::test]
async fn graph_runs_to_completion() {
    let (graph, ctx) = graph().await;
    let result = graph
        .run(
            Counter {
                value: 0,
                history: vec![],
            },
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(result.value, 3);
    assert_eq!(
        result.history,
        vec!["increment", "increment", "increment", "finalize"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn unknown_node_id_errors_clearly() {
    let db = ravn_persistence::Db::open_in_memory().await.unwrap();
    let graph = StateGraph::<Counter>::new("nope", db.clone())
        .with_store(Arc::new(MemoryCheckpointStore::new()))
        .add_node("increment", Increment);
    let ctx = GraphContext::new(db, "trace-1");
    let err = graph
        .run(
            Counter {
                value: 0,
                history: vec![],
            },
            &ctx,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::UnknownNode(ref n) if n == "nope"));
}

#[tokio::test]
async fn checkpoint_round_trip_via_memory_store() {
    let store = MemoryCheckpointStore::new();
    let cp = Checkpoint {
        next_node: "increment",
        state: Counter {
            value: 5,
            history: vec!["a".into(), "b".into()],
        },
    };
    save_typed(&store, "trace-X", None, &cp).await.unwrap();
    let loaded: Checkpoint<Counter> = load_typed(&store, "trace-X").await.unwrap();
    assert_eq!(loaded.state, cp.state);
    assert_eq!(loaded.next_node, "increment");
}

#[tokio::test]
async fn resume_picks_up_where_run_left_off() {
    // Manually save a checkpoint at "finalize" so resume runs only
    // that final node. Asserts that resume actually uses the saved
    // state + next-node rather than starting from `entry`.
    let store = Arc::new(MemoryCheckpointStore::new());
    let cp = Checkpoint {
        next_node: "finalize",
        state: Counter {
            value: 42,
            history: vec!["restored".into()],
        },
    };
    save_typed(&*store, "trace-resume", None, &cp).await.unwrap();

    let db = ravn_persistence::Db::open_in_memory().await.unwrap();
    let graph = StateGraph::<Counter>::new("increment", db.clone())
        .with_store(store)
        .add_node("increment", Increment)
        .add_node("finalize", Finalize);
    let ctx = GraphContext::new(db, "trace-resume");

    let result = graph.resume(&ctx).await.unwrap();
    assert_eq!(result.value, 42); // unchanged by finalize
    assert_eq!(
        result.history,
        vec!["restored".to_string(), "finalize".to_string()]
    );
}

#[tokio::test]
async fn resume_with_end_checkpoint_returns_state_unchanged() {
    let store = Arc::new(MemoryCheckpointStore::new());
    let cp = Checkpoint {
        next_node: END,
        state: Counter {
            value: 99,
            history: vec!["done".into()],
        },
    };
    save_typed(&*store, "trace-end", None, &cp).await.unwrap();

    let db = ravn_persistence::Db::open_in_memory().await.unwrap();
    let graph = StateGraph::<Counter>::new("increment", db.clone())
        .with_store(store)
        .add_node("increment", Increment)
        .add_node("finalize", Finalize);
    let ctx = GraphContext::new(db, "trace-end");

    let result = graph.resume(&ctx).await.unwrap();
    assert_eq!(result.value, 99);
    assert_eq!(result.history, vec!["done".to_string()]);
}

#[tokio::test]
async fn checkpoint_persists_through_db_store() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = ravn_persistence::Db::open(dir.path().join("graph.db"))
        .await
        .unwrap();
    let graph = StateGraph::<Counter>::new("increment", db.clone())
        .add_node("increment", Increment)
        .add_node("finalize", Finalize);
    let ctx = GraphContext::new(db.clone(), "trace-db");

    graph
        .run(
            Counter {
                value: 0,
                history: vec![],
            },
            &ctx,
        )
        .await
        .unwrap();

    // After the run, the latest graph.checkpoint event for this
    // trace_id should encode `next_node == END`.
    let bytes = ravn_persistence::events::latest_payload(&db, "trace-db", "graph.checkpoint")
        .await
        .unwrap()
        .expect("a checkpoint row");
    // Decode just enough to confirm the END marker.
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct WireOnly {
        next_node: String,
    }
    let wire: WireOnly = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(wire.next_node, "__end__");
}
