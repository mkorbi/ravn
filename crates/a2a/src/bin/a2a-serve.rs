//! `a2a-serve` — expose ravn over the A2A (Agent2Agent) protocol.
//!
//! Serves an Agent Card at `/.well-known/agent-card.json` and a JSON-RPC
//! endpoint (`message/send`, `tasks/get`, `tasks/cancel`). An incoming task
//! runs ravn's agent (so this needs an LLM key) with a read-only approver by
//! default. Config: `~/.ravn/a2a.toml`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ravn_a2a::auth::JwtValidator;
use ravn_a2a::config::A2aConfig;
use ravn_a2a::server::{build_card, router, AppState};
use ravn_a2a::task_store::TaskStore;
use ravn_embeddings::Embedder;
use ravn_llm::anthropic::AnthropicProvider;
use ravn_llm::openai::OpenAiProvider;
use ravn_llm::LlmProvider;
use ravn_persistence::Db;
use ravn_tools::{native, ToolRegistry};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let data_dir = data_dir()?;
    let db = Db::open(data_dir.join("state.db"))
        .await
        .context("open state.db")?;
    let (provider, model) = select_provider()?;
    let embedder = Arc::new(Embedder::default_gemma_quiet());

    let mut registry = ToolRegistry::new();
    native::register_defaults(&mut registry, data_dir.clone(), Some(embedder.clone()));
    let tools = Arc::new(registry);

    let config = A2aConfig::load(&data_dir.join("a2a.toml")).await?;
    let card = Arc::new(build_card(&config));

    // Build the JWT validator up front when auth is configured (fetches JWKS).
    let auth = match &config.auth {
        Some(a) => {
            tracing::info!(issuer = %a.issuer, "A2A auth enabled (OIDC/JWT)");
            Some(Arc::new(
                JwtValidator::new(a.clone())
                    .await
                    .context("init JWT validator (JWKS)")?,
            ))
        }
        None => {
            tracing::warn!("A2A auth DISABLED (no [auth] config) — dev only");
            None
        }
    };

    let bind = config.server.bind.clone();
    if config.server.allow_tools.is_empty() {
        tracing::info!("incoming tasks run read-only (no allow_tools configured)");
    } else {
        tracing::info!(allow_tools = ?config.server.allow_tools, "incoming tasks may use these Write/Exec tools");
    }

    let tasks = Arc::new(TaskStore::new());
    let state = AppState {
        provider,
        tools,
        embedder,
        db,
        model,
        data_dir,
        config: Arc::new(config),
        tasks,
        card,
        auth,
    };

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!(%bind, "ravn A2A server listening");
    axum::serve(listener, router(state))
        .await
        .context("axum serve")?;
    Ok(())
}

fn select_provider() -> anyhow::Result<(Arc<dyn LlmProvider>, String)> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        let model = std::env::var("RAVN_MODEL").unwrap_or_else(|_| DEFAULT_ANTHROPIC_MODEL.into());
        let p = AnthropicProvider::from_env().map_err(|e| anyhow::anyhow!("anthropic init: {e}"))?;
        Ok((Arc::new(p), model))
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        let model = std::env::var("RAVN_MODEL").unwrap_or_else(|_| DEFAULT_OPENAI_MODEL.into());
        let p = OpenAiProvider::from_env().map_err(|e| anyhow::anyhow!("openai init: {e}"))?;
        Ok((Arc::new(p), model))
    } else {
        anyhow::bail!("Set ANTHROPIC_API_KEY or OPENAI_API_KEY before running a2a-serve.")
    }
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    Ok(dir.join("ravn"))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .init();
}
