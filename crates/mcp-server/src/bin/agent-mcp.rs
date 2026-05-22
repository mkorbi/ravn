//! `agent-mcp` — serve ravn's read-only tools to MCP clients over stdio.
//!
//! Point Claude Desktop (or `npx @modelcontextprotocol/inspector`) at this
//! binary; it lists + runs the tools named in `~/.ravn/mcp-server.toml`
//! (default: session_search, skill_list, skill_view, datetime) against the
//! user's `state.db`. **All logs go to stderr** — stdout is the JSON-RPC line.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ravn_embeddings::Embedder;
use ravn_mcp_server::config::ExposeConfig;
use ravn_mcp_server::handler::RavnServer;
use ravn_persistence::Db;
use ravn_tools::{native, ToolRegistry};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let data_dir = data_dir()?;
    let db = Db::open(data_dir.join("state.db"))
        .await
        .context("open state.db")?;
    let embedder = Arc::new(Embedder::default_gemma_quiet());

    let mut registry = ToolRegistry::new();
    native::register_defaults(&mut registry, data_dir.clone(), Some(embedder));
    let registry = Arc::new(registry);

    let cfg = ExposeConfig::load(&data_dir.join("mcp-server.toml")).await?;
    let exposed = cfg.resolve_exposed(&registry);
    tracing::info!(tools = ?exposed, "ravn MCP server starting (stdio)");

    let server = RavnServer {
        registry,
        db,
        exposed,
    };
    let running = rmcp::service::serve_server(server, rmcp::transport::io::stdio())
        .await
        .context("serve_server (stdio)")?;
    running.waiting().await.context("server run")?;
    Ok(())
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    Ok(dir.join("ravn"))
}

/// Logs to **stderr only** — stdout carries the MCP JSON-RPC stream.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(io::stderr)
                .with_ansi(false)
                .with_target(false),
        )
        .init();
}
