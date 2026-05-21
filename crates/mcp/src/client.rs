//! Spawn an MCP server as a subprocess and wrap its stdio in an rmcp
//! [`RunningService`]. Returns the peer (cloned by every
//! [`crate::adapter::McpToolAdapter`]) plus a handle that owns the
//! background task — drop the handle to shut the server down.

use std::collections::HashSet;

use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;

use crate::config::ServerConfig;
use crate::error::Error;

pub struct McpConnection {
    pub server: RunningService<RoleClient, ()>,
}

impl McpConnection {
    pub async fn connect(name: &str, server: &ServerConfig) -> Result<Self, Error> {
        tracing::info!(server = name, command = %server.command, "spawning MCP subprocess");

        let mut cmd = Command::new(&server.command);
        cmd.args(&server.args);

        // Env passthrough whitelist. The default (no `env` field) is
        // `PATH` + `HOME` — explicit `env = []` means a fully clean env.
        let allowed: HashSet<&str> = match &server.env {
            Some(list) => list.iter().map(|s| s.as_str()).collect(),
            None => ["PATH", "HOME"].iter().copied().collect(),
        };
        cmd.env_clear();
        for (k, v) in std::env::vars() {
            if allowed.contains(k.as_str()) {
                cmd.env(k, v);
            }
        }

        let transport = TokioChildProcess::new(cmd).map_err(|e| {
            Error::Transport(format!("spawn {}: {e}", server.command))
        })?;

        let service = ()
            .serve(transport)
            .await
            .map_err(|e| Error::Service(format!("handshake: {e}")))?;

        Ok(Self { server: service })
    }

    pub fn peer(&self) -> rmcp::service::Peer<RoleClient> {
        self.server.peer().clone()
    }

    /// Discover every tool the server exposes. Paginated under the hood
    /// via rmcp's `list_all_tools`.
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>, Error> {
        self.peer()
            .list_all_tools()
            .await
            .map_err(|e| Error::Service(format!("list_tools: {e}")))
    }

    pub async fn close(mut self) {
        let _ = self.server.close().await;
    }
}
