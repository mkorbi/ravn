//! MCP client integration (Phase 2.1, D14).
//!
//! Reads `~/.ravn/mcp-servers.toml`, spawns each configured server as a
//! subprocess, discovers its tools via the MCP `list_tools` RPC, and
//! registers each one as a [`ravn_tools::Tool`]-trait impl
//! ([`McpToolAdapter`]). Tool names are namespaced `<server>__<tool>`
//! to avoid collisions between servers; per-tool permission overrides
//! in the config trump per-server defaults.
//!
//! `connect_and_register` returns a `Vec<McpConnection>` — the caller
//! (cli) holds them for the program's lifetime; dropping closes the
//! subprocess.

pub mod adapter;
pub mod client;
pub mod config;
pub mod error;

use std::path::Path;

pub use adapter::McpToolAdapter;
pub use client::McpConnection;
pub use config::Config;
pub use error::Error;

use ravn_tools::ToolRegistry;

/// Read the config file at `path`, connect to every listed server, and
/// register all discovered tools (namespaced) into `reg`. Returns the
/// owned connections — keep them alive for the program lifetime;
/// dropping closes the subprocesses.
///
/// Per-server connect failures log a warning and skip that server
/// (rather than aborting startup) — the user might have an offline
/// server they're not actively using.
pub async fn connect_and_register(
    config_path: &Path,
    reg: &mut ToolRegistry,
) -> Result<Vec<McpConnection>, Error> {
    let cfg = Config::load(config_path).await?;
    let mut connections = Vec::new();

    for (server_name, server_cfg) in cfg.servers.iter() {
        match McpConnection::connect(server_name, server_cfg).await {
            Ok(conn) => match conn.list_tools().await {
                Ok(tools) => {
                    let n = tools.len();
                    for t in tools {
                        let namespaced = format!("{server_name}__{}", t.name);
                        let permission = cfg.permission_for(server_name, t.name.as_ref());
                        let schema = serde_json::Value::Object(t.input_schema.as_ref().clone());
                        let adapter = McpToolAdapter {
                            name: Box::leak(namespaced.clone().into_boxed_str()),
                            description: Box::leak(
                                t.description
                                    .as_ref()
                                    .map(|d| d.to_string())
                                    .unwrap_or_else(|| t.name.to_string())
                                    .into_boxed_str(),
                            ),
                            permission,
                            schema,
                            remote_name: t.name.to_string(),
                            peer: conn.peer(),
                        };
                        reg.register_arc(std::sync::Arc::new(adapter));
                    }
                    tracing::info!(server = %server_name, registered = n, "MCP server connected");
                    connections.push(conn);
                }
                Err(e) => {
                    tracing::warn!(server = %server_name, error = %e, "list_tools failed");
                    conn.close().await;
                }
            },
            Err(e) => {
                tracing::warn!(server = %server_name, error = %e, "MCP connect failed");
            }
        }
    }

    Ok(connections)
}
