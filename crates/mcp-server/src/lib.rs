//! ravn as an MCP **server** (Phase 5.1/5.2).
//!
//! Flips the Phase-2 relationship: instead of consuming external MCP servers,
//! ravn exposes its **own** read-only tools (session-search, skills, datetime)
//! to external MCP clients (Claude Desktop, the MCP Inspector) over stdio.
//! The binary lives in `src/bin/agent-mcp.rs`.

pub mod config;
pub mod handler;

pub use config::ExposeConfig;
pub use handler::RavnServer;
