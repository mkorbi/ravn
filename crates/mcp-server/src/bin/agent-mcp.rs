//! `agent-mcp` — serve ravn's read-only tools to MCP clients.
//!
//! Point Claude Desktop (or `npx @modelcontextprotocol/inspector`) at this
//! binary; it lists + runs the tools named in `~/.ravn/mcp-server.toml`
//! (default: session_search, skill_list, skill_view, datetime) against the
//! user's `state.db`. stdio is on by default; enable `[http]` for a networked
//! Streamable-HTTP endpoint (Bearer/IP auth in `[http]`). **All logs go to
//! stderr** — stdout is the JSON-RPC line.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ravn_embeddings::Embedder;
use ravn_mcp_server::config::ExposeConfig;
use ravn_mcp_server::{serve_http, serve_stdio, AuthConfig, RavnServer};
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
    tracing::info!(
        tools = ?exposed,
        stdio = cfg.stdio.enabled,
        http = cfg.http.enabled,
        "ravn MCP server starting"
    );

    let server = RavnServer {
        registry,
        db,
        exposed,
    };
    let auth = AuthConfig::from_http(&cfg.http);
    if cfg.http.enabled {
        warn_if_open(&cfg.http.bind, &auth);
    }

    match (cfg.stdio.enabled, cfg.http.enabled) {
        (true, true) => {
            let http_server = server.clone();
            let bind = cfg.http.bind.clone();
            tokio::try_join!(
                serve_stdio(server),
                serve_http(&bind, auth, move || http_server.clone()),
            )?;
        }
        (true, false) => serve_stdio(server).await?,
        (false, true) => serve_http(&cfg.http.bind, auth, move || server.clone()).await?,
        (false, false) => anyhow::bail!(
            "no transport enabled: set [stdio].enabled or [http].enabled in mcp-server.toml"
        ),
    }
    Ok(())
}

/// Warn when HTTP is bound to a non-loopback address with no auth configured.
fn warn_if_open(bind: &str, auth: &AuthConfig) {
    let non_loopback = bind
        .parse::<SocketAddr>()
        .map(|a| !a.ip().is_loopback())
        .unwrap_or(false);
    if non_loopback && !auth.is_enabled() {
        tracing::warn!(%bind, "HTTP bound non-loopback with no auth — set http.bearer_token and/or http.ip_allowlist");
    }
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
