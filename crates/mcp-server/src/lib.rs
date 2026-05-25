//! ravn as an MCP **server** (Phase 5.1/5.2/5.3).
//!
//! Flips the Phase-2 relationship: instead of consuming external MCP servers,
//! ravn exposes its **own** read-only tools (session-search, skills, datetime)
//! to external MCP clients (Claude Desktop, the MCP Inspector). Two transports
//! (Phase 5.1): **stdio** (default, for subprocess clients) and **Streamable
//! HTTP** (axum-mounted at `/mcp`), the latter gated by Bearer + IP auth
//! (Phase 5.3). Only `Permission::Read` tools are ever exposed (see [`config`]),
//! so an external client can never make ravn write or run anything.
//! The binary lives in `src/bin/agent-mcp.rs`.

pub mod auth;
pub mod config;
pub mod handler;

pub use auth::AuthConfig;
pub use config::ExposeConfig;
pub use handler::RavnServer;

use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::ServiceExt;

/// Mount path for the Streamable HTTP transport.
pub const HTTP_MCP_PATH: &str = "/mcp";

/// Serve over stdio until the client disconnects. Blocks the current task.
/// All logging must go to stderr (the binary configures this) — stdout carries
/// the JSON-RPC frames.
pub async fn serve_stdio(server: RavnServer) -> anyhow::Result<()> {
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

/// Serve over Streamable HTTP at `bind`, mounting the MCP service at
/// [`HTTP_MCP_PATH`] behind the [`AuthConfig`] middleware (Phase 5.3).
/// `factory` produces a fresh [`RavnServer`] per session (cheap — they share
/// `Arc`/`Db` clones).
///
/// rmcp's default config restricts the inbound `Host` header to loopback
/// (DNS-rebinding defense), so an unauthenticated server is safe on
/// `127.0.0.1`; [`http_server_config`] relaxes it for non-loopback binds, where
/// the operator's Bearer/IP auth carries the load.
pub async fn serve_http(
    bind: &str,
    auth: AuthConfig,
    factory: impl Fn() -> RavnServer + Send + Sync + 'static,
) -> anyhow::Result<()> {
    let service = StreamableHttpService::new(
        move || Ok(factory()),
        Arc::new(LocalSessionManager::default()),
        http_server_config(bind),
    );

    let auth = Arc::new(auth);
    let app = axum::Router::new()
        .route_service(HTTP_MCP_PATH, service)
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            auth::require_auth,
        ));

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, path = HTTP_MCP_PATH, auth = auth.is_enabled(), "MCP HTTP transport listening");
    // `ConnectInfo::<SocketAddr>` is required by the IP-allowlist check.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// rmcp `Host`-header allowlist policy per bind: loopback → default
/// (loopback-only); unspecified (`0.0.0.0`) → disabled (auth must gate); a
/// specific non-loopback IP → accept that authority.
fn http_server_config(bind: &str) -> StreamableHttpServerConfig {
    let cfg = StreamableHttpServerConfig::default();
    match bind.parse::<SocketAddr>() {
        Ok(addr) if addr.ip().is_loopback() => cfg,
        Ok(addr) if addr.ip().is_unspecified() => cfg.disable_allowed_hosts(),
        Ok(addr) => cfg.with_allowed_hosts([bind.to_string(), addr.ip().to_string()]),
        Err(_) => cfg,
    }
}
