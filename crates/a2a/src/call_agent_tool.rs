//! `call_agent` — a ravn tool that delegates a task to an external A2A peer.
//!
//! Registered by the cli (not in `native::register_defaults`), so only the
//! interactive agent gets it — the A2A server itself doesn't, which avoids
//! agents calling each other in a loop.

use std::sync::Arc;

use async_trait::async_trait;
use ravn_tools::{Permission, Tool, ToolContext, ToolError, ToolOutput};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::client::A2aClient;
use crate::config::A2aConfig;

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Name of a configured A2A peer (from `~/.ravn/a2a.toml`).
    peer: String,
    /// The task / message to send to that agent.
    message: String,
}

pub struct CallAgentTool {
    config: Arc<A2aConfig>,
}

impl CallAgentTool {
    pub fn new(config: Arc<A2aConfig>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CallAgentTool {
    fn name(&self) -> &'static str {
        "call_agent"
    }
    fn description(&self) -> &'static str {
        "Delegate a task to an external A2A (Agent2Agent) peer configured in ~/.ravn/a2a.toml, and return its reply. Use this to reach capabilities other agents provide. Requires approval."
    }
    fn permission(&self) -> Permission {
        Permission::Write
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(Args)).unwrap_or_default()
    }

    async fn invoke(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: Args =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let peer = self.config.peer(&args.peer).ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "unknown A2A peer '{}' (configure it under [[peer]] in a2a.toml)",
                args.peer
            ))
        })?;
        let reply = A2aClient::new()
            .call_peer(peer, &args.message)
            .await
            .map_err(|e| ToolError::Internal(e.to_string()))?;
        // Replies from external agents are untrusted input.
        Ok(ToolOutput::untrusted(reply))
    }
}
