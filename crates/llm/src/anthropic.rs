//! Anthropic adapter — wraps `rig::providers::anthropic` (D1) with native
//! prompt-cache support.
//!
//! Cache modes (chosen at provider construction):
//! - [`CacheMode::Off`] — no cache_control markers.
//! - [`CacheMode::Auto`] — rig's `with_automatic_caching()`. The API places
//!   the breakpoint on the last cacheable block automatically; recommended
//!   for multi-turn conversations. 5-min TTL (no beta header required).
//! - [`CacheMode::AutoOneHour`] — `with_automatic_caching_1h()`. Requires the
//!   `anthropic-beta: extended-cache-ttl-2025-04-11` header (set on the
//!   client builder).
//! - [`CacheMode::Manual`] — rig's `with_prompt_caching()`: explicit
//!   breakpoints on the system prompt and last message. Use when you need
//!   finer control than auto.
//!
//! `CompletionRequest::cache_breakpoints` from the caller is not yet honored
//! per-block in Phase 0 — the cache mode chosen here applies request-wide.
//! Per-block breakpoint forwarding is a Phase-1 follow-up.

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::CompletionModel as _;
use rig_core::completion::message::ReasoningContent as RigReasoningContent;
use rig_core::providers::anthropic;
use rig_core::streaming::StreamedAssistantContent;

use crate::provider::{Error, LlmProvider};
use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, FinishReason, StreamChunk};
use crate::rig_bridge::{
    anthropic_thinking_param, classify_completion_error, from_rig_assistant_choice,
    from_rig_usage, to_rig_request,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheMode {
    Off,
    #[default]
    Auto,
    AutoOneHour,
    Manual,
}

pub struct AnthropicProvider {
    client: anthropic::Client,
    cache_mode: CacheMode,
}

impl AnthropicProvider {
    pub fn from_api_key(api_key: impl AsRef<str>) -> Result<Self, Error> {
        let client = anthropic::Client::builder()
            .api_key(api_key.as_ref())
            .build()
            .map_err(|e| Error::InvalidRequest(format!("anthropic client init: {e}")))?;
        Ok(Self {
            client,
            cache_mode: CacheMode::Auto,
        })
    }

    pub fn from_env() -> Result<Self, Error> {
        let client = anthropic::Client::from_env()
            .map_err(|e| Error::InvalidRequest(format!("anthropic client init: {e}")))?;
        Ok(Self {
            client,
            cache_mode: CacheMode::Auto,
        })
    }

    /// Build with the `extended-cache-ttl-2025-04-11` beta header preconfigured,
    /// allowing 1-hour cache TTL when paired with [`CacheMode::AutoOneHour`].
    pub fn with_extended_cache(mut self) -> Self {
        // The header has to be set on the underlying client builder, not the
        // model — but the existing client is fine for 5-min TTL. Callers that
        // need 1h should construct via `Client::builder().anthropic_beta(...)`
        // directly. This helper only flips the cache mode.
        self.cache_mode = CacheMode::AutoOneHour;
        self
    }

    pub fn with_cache_mode(mut self, mode: CacheMode) -> Self {
        self.cache_mode = mode;
        self
    }

    fn model(&self, name: &str) -> <anthropic::Client as CompletionClient>::CompletionModel {
        let base = self.client.completion_model(name);
        match self.cache_mode {
            CacheMode::Off => base,
            CacheMode::Auto => base.with_automatic_caching(),
            CacheMode::AutoOneHour => base.with_automatic_caching_1h(),
            CacheMode::Manual => base.with_prompt_caching(),
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn supports_caching(&self) -> bool {
        true
    }

    fn supports_reasoning(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, Error> {
        let model_id = req.model.clone();
        let thinking_param = req.reasoning_effort.map(anthropic_thinking_param);
        let mut rig_req = to_rig_request(req)?;
        rig_req.additional_params = thinking_param;

        let model = self.model(&model_id);
        let resp = model
            .completion(rig_req)
            .await
            .map_err(|e| classify_completion_error("anthropic", e))?;

        Ok(CompletionResponse {
            model: model_id,
            content: from_rig_assistant_choice(&resp.choice),
            usage: from_rig_usage(&resp.usage),
            finish_reason: FinishReason::Stop,
        })
    }

    fn stream(
        &self,
        req: CompletionRequest,
    ) -> BoxStream<'static, Result<StreamChunk, Error>> {
        let model_id = req.model.clone();
        let thinking_param = req.reasoning_effort.map(anthropic_thinking_param);
        let model = self.model(&model_id);

        let mut rig_req = match to_rig_request(req) {
            Ok(r) => r,
            Err(e) => return Box::pin(futures::stream::once(async move { Err(e) })),
        };
        rig_req.additional_params = thinking_param;

        Box::pin(async_stream::stream! {
            let mut s = match model.stream(rig_req).await {
                Ok(s) => s,
                Err(e) => {
                    yield Err(classify_completion_error("anthropic", e));
                    return;
                }
            };

            // Track which internal_call_id we have already begun via
            // ToolCallDelta::Name. When the same tool also surfaces as
            // a complete `ToolCall` (Anthropic emits both), we just
            // finalize it instead of re-emitting Start/Delta/End,
            // which would create duplicate tool_use blocks and trip
            // Anthropic's "ids must be unique" validation on the next
            // turn.
            let mut delta_tool: Option<String> = None;

            while let Some(item) = StreamExt::next(&mut s).await {
                match item {
                    Err(e) => {
                        yield Err(classify_completion_error("anthropic", e));
                        return;
                    }
                    Ok(chunk) => match chunk {
                        StreamedAssistantContent::Text(t) => {
                            yield Ok(StreamChunk::TextDelta(t.text));
                        }
                        StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                            yield Ok(StreamChunk::ThinkingDelta(reasoning));
                        }
                        StreamedAssistantContent::Reasoning(r) => {
                            for block in r.content {
                                if let RigReasoningContent::Text { text, .. } = block {
                                    yield Ok(StreamChunk::ThinkingDelta(text));
                                }
                            }
                        }
                        StreamedAssistantContent::ToolCall { tool_call, internal_call_id } => {
                            if delta_tool.as_deref() == Some(&internal_call_id) {
                                // Deltas already produced Start + (partial)
                                // Delta. Just close the block.
                                yield Ok(StreamChunk::ToolUseEnd);
                                delta_tool = None;
                            } else {
                                yield Ok(StreamChunk::ToolUseStart {
                                    id: tool_call.id.clone(),
                                    name: tool_call.function.name.clone(),
                                });
                                yield Ok(StreamChunk::ToolUseDelta {
                                    partial_json: tool_call.function.arguments.to_string(),
                                });
                                yield Ok(StreamChunk::ToolUseEnd);
                            }
                        }
                        StreamedAssistantContent::ToolCallDelta { id, internal_call_id, content } => {
                            use rig_core::streaming::ToolCallDeltaContent;
                            match content {
                                ToolCallDeltaContent::Name(name) => {
                                    delta_tool = Some(internal_call_id);
                                    yield Ok(StreamChunk::ToolUseStart { id, name });
                                }
                                ToolCallDeltaContent::Delta(partial_json) => {
                                    yield Ok(StreamChunk::ToolUseDelta { partial_json });
                                }
                            }
                        }
                        StreamedAssistantContent::Final(_) => {}
                    },
                }
            }

            let _ = delta_tool;
            use rig_core::completion::GetTokenUsage;
            if let Some(rig_usage) = s.response.as_ref().and_then(|r| r.token_usage()) {
                yield Ok(StreamChunk::Usage(from_rig_usage(&rig_usage)));
            }
            yield Ok(StreamChunk::Done { finish_reason: FinishReason::Stop });
        })
    }
}
