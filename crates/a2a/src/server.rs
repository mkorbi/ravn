//! axum router exposing ravn over A2A: the Agent Card + JSON-RPC endpoint.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use ravn_embeddings::Embedder;
use ravn_llm::LlmProvider;
use ravn_persistence::Db;
use ravn_tools::ToolRegistry;
use serde_json::Value;
use uuid::Uuid;

use crate::agent_runner::{run_to_completion, RunnerCtx};
use crate::config::{A2aConfig, ServerConfig};
use crate::task_store::TaskStore;
use crate::types::{
    error_code, AgentCapabilities, AgentCard, AgentSkill, JsonRpcRequest, JsonRpcResponse,
    MessageSendParams, TaskIdParams,
};

#[derive(Clone)]
pub struct AppState {
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub embedder: Arc<Embedder>,
    pub db: Db,
    pub model: String,
    pub data_dir: PathBuf,
    pub config: Arc<A2aConfig>,
    pub tasks: Arc<TaskStore>,
    pub card: Arc<AgentCard>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/.well-known/agent-card.json", get(agent_card))
        .route("/", post(jsonrpc))
        .with_state(state)
}

async fn agent_card(State(state): State<AppState>) -> Json<AgentCard> {
    Json((*state.card).clone())
}

async fn jsonrpc(
    State(state): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let id = req.id.clone();
    let resp = match req.method.as_str() {
        "message/send" => handle_message_send(&state, id, req.params).await,
        "tasks/get" => handle_tasks_get(&state, id, req.params),
        "tasks/cancel" => handle_tasks_cancel(&state, id, req.params),
        other => JsonRpcResponse::err(
            id,
            error_code::METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        ),
    };
    Json(resp)
}

async fn handle_message_send(state: &AppState, id: Value, params: Value) -> JsonRpcResponse {
    let params: MessageSendParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::err(id, error_code::INVALID_PARAMS, format!("invalid params: {e}"))
        }
    };
    let context_id = params
        .message
        .context_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let task_id = Uuid::new_v4().to_string();
    state.tasks.create(&task_id, &context_id);

    let runner = RunnerCtx {
        provider: state.provider.clone(),
        tools: state.tools.clone(),
        embedder: state.embedder.clone(),
        db: state.db.clone(),
        model: state.model.clone(),
        data_dir: state.data_dir.clone(),
        allow_tools: state.config.server.allow_tools.clone(),
        tasks: state.tasks.clone(),
    };
    let task = run_to_completion(&runner, &params.message, &task_id, &context_id).await;
    JsonRpcResponse::ok(id, task)
}

fn handle_tasks_get(state: &AppState, id: Value, params: Value) -> JsonRpcResponse {
    let params: TaskIdParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::err(id, error_code::INVALID_PARAMS, format!("invalid params: {e}"))
        }
    };
    match state.tasks.get(&params.id) {
        Some(t) => JsonRpcResponse::ok(id, t),
        None => JsonRpcResponse::err(id, error_code::TASK_NOT_FOUND, "task not found"),
    }
}

fn handle_tasks_cancel(state: &AppState, id: Value, params: Value) -> JsonRpcResponse {
    let params: TaskIdParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return JsonRpcResponse::err(id, error_code::INVALID_PARAMS, format!("invalid params: {e}"))
        }
    };
    match state.tasks.cancel(&params.id) {
        Some(t) => JsonRpcResponse::ok(id, t),
        None => JsonRpcResponse::err(id, error_code::TASK_NOT_FOUND, "task not found"),
    }
}

/// Build the Agent Card advertised at `/.well-known/agent-card.json`.
pub fn build_card(cfg: &ServerConfig) -> AgentCard {
    AgentCard {
        protocol_version: "0.3.0".to_string(),
        name: cfg.name.clone(),
        description: cfg.description.clone(),
        url: cfg.public_url.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: AgentCapabilities::default(),
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        skills: vec![AgentSkill {
            id: "general-assistant".to_string(),
            name: "General assistant".to_string(),
            description:
                "Answers questions and performs read-only tasks using ravn's tools, memory, and skills."
                    .to_string(),
            tags: vec!["assistant".to_string(), "search".to_string()],
            examples: vec!["What did we discuss about the deploy last week?".to_string()],
        }],
        security_schemes: Default::default(),
        security: vec![],
    }
}
