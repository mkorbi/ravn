//! ravn as an **A2A (Agent2Agent) peer** (Phase 5.4 / 5.5 / 5.7).
//!
//! - [`types`]: hand-rolled protocol structs (Agent Card, Message, Task, …).
//! - [`config`]: `~/.ravn/a2a.toml` (server, auth, peers).
//! - server modules (task store, agent runner, axum router, auth) and the
//!   client + `call_agent` tool are added stage by stage.

pub mod agent_runner;
pub mod config;
pub mod server;
pub mod task_store;
pub mod types;
