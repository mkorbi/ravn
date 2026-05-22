//! HTTP-surface tests for the A2A server: Agent Card + JSON-RPC error paths.
//! These never invoke the LLM (only `message/send` does), so they run with a
//! mock provider and no API key.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::stream::BoxStream;
use futures::StreamExt;
use ravn_a2a::config::A2aConfig;
use ravn_a2a::server::{build_card, router, AppState};
use ravn_a2a::task_store::TaskStore;
use ravn_a2a::types::AgentCard;
use ravn_embeddings::Embedder;
use ravn_llm::provider::Error as LlmError;
use ravn_llm::request::CompletionRequest;
use ravn_llm::response::{CompletionResponse, StreamChunk};
use ravn_llm::LlmProvider;
use ravn_persistence::Db;
use ravn_tools::ToolRegistry;
use serde_json::{json, Value};
use tower::ServiceExt;

struct MockProvider;

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn supports_caching(&self) -> bool {
        false
    }
    fn supports_reasoning(&self) -> bool {
        false
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::Transport("mock".into()))
    }
    fn stream(
        &self,
        _req: CompletionRequest,
    ) -> BoxStream<'static, Result<StreamChunk, LlmError>> {
        futures::stream::once(async { Err(LlmError::Transport("mock".into())) }).boxed()
    }
}

async fn test_state() -> AppState {
    let db = Db::open_in_memory().await.unwrap();
    let config = Arc::new(A2aConfig::default());
    let card = Arc::new(build_card(&config.server));
    AppState {
        provider: Arc::new(MockProvider),
        tools: Arc::new(ToolRegistry::new()),
        embedder: Arc::new(Embedder::default_gemma_quiet()),
        db,
        model: "mock".into(),
        data_dir: std::env::temp_dir(),
        config,
        tasks: Arc::new(TaskStore::new()),
        card,
    }
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn serves_agent_card() {
    let app = router(test_state().await);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent-card.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let card: AgentCard = serde_json::from_value(body_json(resp).await).unwrap();
    assert_eq!(card.name, "ravn");
    assert_eq!(card.protocol_version, "0.3.0");
    assert!(!card.skills.is_empty());
}

async fn post_rpc(app: axum::Router, payload: Value) -> Value {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

#[tokio::test]
async fn unknown_method_is_method_not_found() {
    let v = post_rpc(
        router(test_state().await),
        json!({"jsonrpc":"2.0","id":1,"method":"bogus/method","params":{}}),
    )
    .await;
    assert_eq!(v["error"]["code"], -32601);
}

#[tokio::test]
async fn tasks_get_unknown_is_task_not_found() {
    let v = post_rpc(
        router(test_state().await),
        json!({"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{"id":"nope"}}),
    )
    .await;
    assert_eq!(v["error"]["code"], -32001);
}
