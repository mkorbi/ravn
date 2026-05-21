//! Agent-Loop, Reasoning-Router, Budgets.
//!
//! Phase 1 ships the ReAct-Loop ([`agent::Agent`]). Reasoning-Router
//! (Fast/Deep mode) folgt in Phase 3.

pub mod agent;
pub mod budget;
pub mod error;
pub mod event;
pub mod reasoning;

pub use agent::{Agent, AgentConfig, RunContext, RunSummary};
pub use budget::{Budget, BudgetTracker, BudgetUsage};
pub use error::AgentError;
pub use event::{ChannelSink, EventSink, LoopEvent, NullSink};
pub use reasoning::Mode as ReasoningMode;

#[cfg(test)]
mod tests;
