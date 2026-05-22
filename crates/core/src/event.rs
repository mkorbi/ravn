//! Events emitted by the ReAct-Loop. Frontends (TUI, MCP server, tests)
//! implement [`EventSink`] to receive them.

use async_trait::async_trait;
use ravn_llm::Usage;
use ravn_tools::Permission;
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum LoopEvent {
    StepStart {
        step: usize,
    },
    /// Router picked a [`crate::ReasoningMode`] for this step.
    ModeChange {
        step: usize,
        mode: crate::reasoning::Mode,
    },
    TextDelta(String),
    /// Extended Thinking delta — caller may surface or drop.
    ThinkingDelta(String),
    ToolStart {
        id: String,
        name: String,
        args: Value,
        permission: Permission,
    },
    ToolEnd {
        id: String,
        name: String,
        content: String,
        is_error: bool,
        trustworthy: bool,
    },
    ToolDenied {
        id: String,
        name: String,
    },
    Usage(Usage),
    BudgetExceeded {
        reason: String,
    },
    Done,
    Error(String),
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: LoopEvent);
}

/// Sink that discards all events. Used in tests and when the caller only
/// cares about the final [`crate::agent::RunSummary`].
pub struct NullSink;

#[async_trait]
impl EventSink for NullSink {
    async fn emit(&self, _event: LoopEvent) {}
}

/// Sink backed by an mpsc channel — the most common production pattern.
pub struct ChannelSink {
    tx: tokio::sync::mpsc::Sender<LoopEvent>,
}

impl ChannelSink {
    pub fn new(tx: tokio::sync::mpsc::Sender<LoopEvent>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl EventSink for ChannelSink {
    async fn emit(&self, event: LoopEvent) {
        let _ = self.tx.send(event).await;
    }
}
