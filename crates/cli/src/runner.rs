//! Spawns the LLM streaming task and forwards chunks back to the UI.

use std::sync::Arc;

use futures::StreamExt;
use ravn_llm::{
    anthropic::AnthropicProvider, openai::OpenAiProvider, LlmProvider, Message, PromptBuilder,
    StreamChunk,
};
use ravn_persistence::{events, messages, Db};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::UiUpdate;

pub enum Provider {
    Anthropic(AnthropicProvider),
    OpenAi(OpenAiProvider),
}

impl Provider {
    pub fn stream(
        &self,
        req: ravn_llm::CompletionRequest,
    ) -> futures::stream::BoxStream<'static, Result<StreamChunk, ravn_llm::Error>> {
        match self {
            Provider::Anthropic(p) => p.stream(req),
            Provider::OpenAi(p) => p.stream(req),
        }
    }
}

pub struct RunArgs {
    pub db: Db,
    pub session_id: String,
    pub model: String,
    pub system_prompt: String,
    pub history: Vec<Message>,
    pub user_turn: Message,
    pub max_tokens: u32,
}

/// Spawn the LLM call. Returns immediately with a CancellationToken the UI
/// can fire to stop the stream.
pub fn spawn_completion(
    provider: Arc<Provider>,
    args: RunArgs,
    tx: Arc<mpsc::Sender<UiUpdate>>,
) -> CancellationToken {
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    tokio::spawn(async move {
        let RunArgs {
            db,
            session_id,
            model,
            system_prompt,
            history,
            user_turn,
            max_tokens,
        } = args;

        // Persist the user turn first.
        let user_json = match serde_json::to_string(&user_turn.content) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(UiUpdate::Error(format!("encode user turn: {e}"))).await;
                return;
            }
        };
        if let Err(e) = messages::append(&db, &session_id, "user", &user_json).await {
            let _ = tx
                .send(UiUpdate::Error(format!("persist user msg: {e}")))
                .await;
            return;
        }

        let trace_id = uuid::Uuid::new_v4().to_string();
        let _ = events::append_json(
            &db,
            Some(&trace_id),
            Some(&session_id),
            "llm.request",
            &serde_json::json!({
                "model": model,
                "history_len": history.len(),
            }),
        )
        .await;

        let req = PromptBuilder::new()
            .system(system_prompt)
            .history(history)
            .build(&model, user_turn, max_tokens);

        let mut stream = provider.stream(req);
        let mut full_text = String::new();

        loop {
            tokio::select! {
                biased;
                _ = cancel_clone.cancelled() => {
                    let _ = tx.send(UiUpdate::Error("cancelled".into())).await;
                    return;
                }
                next = stream.next() => {
                    match next {
                        None => break,
                        Some(Err(e)) => {
                            let _ = tx.send(UiUpdate::Error(e.to_string())).await;
                            return;
                        }
                        Some(Ok(chunk)) => match chunk {
                            StreamChunk::TextDelta(t) => {
                                full_text.push_str(&t);
                                if tx.send(UiUpdate::TextDelta(t)).await.is_err() {
                                    return;
                                }
                            }
                            StreamChunk::ThinkingDelta(t) => {
                                let _ = tx.send(UiUpdate::ThinkingDelta(t)).await;
                            }
                            StreamChunk::Usage(u) => {
                                let _ = tx.send(UiUpdate::Usage(u)).await;
                            }
                            StreamChunk::Done { .. } => break,
                            StreamChunk::ToolUseStart { .. }
                            | StreamChunk::ToolUseDelta { .. }
                            | StreamChunk::ToolUseEnd => {
                                // Phase 0 has no tools; ignore.
                            }
                        }
                    }
                }
            }
        }

        if !full_text.is_empty() {
            let blocks = ravn_llm::Message::assistant(full_text.clone()).content;
            let assistant_json =
                serde_json::to_string(&blocks).unwrap_or_else(|_| "[]".to_string());
            let _ = messages::append(&db, &session_id, "assistant", &assistant_json).await;
        }

        let _ = events::append_json(
            &db,
            Some(&trace_id),
            Some(&session_id),
            "llm.response",
            &serde_json::json!({"len": full_text.len()}),
        )
        .await;

        let _ = tx.send(UiUpdate::Done).await;
    });

    cancel
}
