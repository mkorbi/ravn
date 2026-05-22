//! axum router exposing ravn over A2A: the Agent Card + JSON-RPC endpoint.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ravn_embeddings::Embedder;
use ravn_llm::LlmProvider;
use ravn_persistence::Db;
use ravn_tools::ToolRegistry;
use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent_runner::{run_streaming, run_to_completion, RunnerCtx};
use crate::auth::JwtValidator;
use crate::config::A2aConfig;
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
    /// Some ⇒ JSON-RPC requests require a valid bearer JWT (the Agent Card
    /// stays public so clients can discover the auth requirement).
    pub auth: Option<Arc<JwtValidator>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/.well-known/agent-card.json", get(agent_card))
        .route("/", post(jsonrpc))
        .layer(from_fn_with_state(state.clone(), require_auth))
        .with_state(state)
}

/// Gate JSON-RPC requests on a valid bearer JWT when `[auth]` is configured.
/// The Agent Card (`/.well-known/…`) is always public.
async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if req.uri().path().starts_with("/.well-known/") {
        return next.run(req).await;
    }
    let Some(validator) = state.auth.clone() else {
        return next.run(req).await; // auth disabled (dev)
    };
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")));
    match token {
        Some(t) => match validator.validate(t).await {
            Ok(()) => next.run(req).await,
            Err(e) => (StatusCode::UNAUTHORIZED, e).into_response(),
        },
        None => (StatusCode::UNAUTHORIZED, "missing bearer token".to_string()).into_response(),
    }
}

async fn agent_card(State(state): State<AppState>) -> Json<AgentCard> {
    Json((*state.card).clone())
}

async fn jsonrpc(State(state): State<AppState>, Json(req): Json<JsonRpcRequest>) -> Response {
    let id = req.id.clone();
    match req.method.as_str() {
        "message/send" => Json(handle_message_send(&state, id, req.params).await).into_response(),
        "message/stream" => handle_message_stream(&state, id, req.params).await,
        "tasks/get" => Json(handle_tasks_get(&state, id, req.params)).into_response(),
        "tasks/cancel" => Json(handle_tasks_cancel(&state, id, req.params)).into_response(),
        other => Json(JsonRpcResponse::err(
            id,
            error_code::METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        ))
        .into_response(),
    }
}

fn make_runner(state: &AppState) -> RunnerCtx {
    RunnerCtx {
        provider: state.provider.clone(),
        tools: state.tools.clone(),
        embedder: state.embedder.clone(),
        db: state.db.clone(),
        model: state.model.clone(),
        data_dir: state.data_dir.clone(),
        allow_tools: state.config.server.allow_tools.clone(),
        tasks: state.tasks.clone(),
    }
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

    let task = run_to_completion(&make_runner(state), &params.message, &task_id, &context_id).await;
    JsonRpcResponse::ok(id, task)
}

/// `message/stream`: run the agent and stream A2A events back over SSE.
async fn handle_message_stream(state: &AppState, id: Value, params: Value) -> Response {
    let params: MessageSendParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return Json(JsonRpcResponse::err(
                id,
                error_code::INVALID_PARAMS,
                format!("invalid params: {e}"),
            ))
            .into_response()
        }
    };
    let context_id = params
        .message
        .context_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let task_id = Uuid::new_v4().to_string();
    state.tasks.create(&task_id, &context_id);

    let (tx, mut rx) = mpsc::channel::<Value>(64);

    // First SSE frame: the task in `working` state.
    state.tasks.set_state(&task_id, crate::types::TaskState::Working);
    if let Some(task) = state.tasks.get(&task_id) {
        let _ = tx
            .send(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": task}))
            .await;
    }

    // Drive the agent on a background task; it streams events into `tx`.
    let runner = make_runner(state);
    let rpc_id = id.clone();
    let message = params.message;
    let tid = task_id.clone();
    let cid = context_id.clone();
    tokio::spawn(async move {
        run_streaming(&runner, &message, &tid, &cid, rpc_id, tx).await;
    });

    let stream = async_stream::stream! {
        while let Some(v) = rx.recv().await {
            yield Ok::<_, std::convert::Infallible>(Event::default().data(v.to_string()));
        }
    };
    Sse::new(stream).into_response()
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
pub fn build_card(cfg: &A2aConfig) -> AgentCard {
    let mut security_schemes = std::collections::BTreeMap::new();
    let mut security = Vec::new();
    if let Some(auth) = &cfg.auth {
        security_schemes.insert(
            "oidc".to_string(),
            serde_json::json!({
                "type": "openIdConnect",
                "openIdConnectUrl": format!(
                    "{}/.well-known/openid-configuration",
                    auth.issuer.trim_end_matches('/')
                ),
            }),
        );
        let mut req = std::collections::BTreeMap::new();
        req.insert("oidc".to_string(), auth.required_scopes.clone());
        security.push(req);
    }
    AgentCard {
        protocol_version: "0.3.0".to_string(),
        name: cfg.server.name.clone(),
        description: cfg.server.description.clone(),
        url: cfg.server.public_url.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: AgentCapabilities {
            streaming: true,
            ..AgentCapabilities::default()
        },
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
        security_schemes,
        security,
    }
}
