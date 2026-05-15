//! OpenAI adapter — wraps `rig::providers::openai` (D1).
//!
//! Phase 0.4 scope: text-only completions and streaming. Tool calls,
//! ContentBlock::ToolUse/ToolResult, and ContentBlock::Thinking translation
//! are stubbed with `Error::InvalidRequest` and will be filled in alongside
//! Phase 1 (native tools) and Phase 3 (reasoning router).
//!
//! Cache breakpoints are accepted but ignored — OpenAI manages its prompt
//! cache automatically.

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{BoxStream, Stream, StreamExt};

use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::{
    CompletionModel as _, CompletionRequest as RigCompletionRequest, Message as RigMessage,
    ToolDefinition as RigToolDefinition,
};
use rig_core::completion::message::{
    AssistantContent as RigAssistantContent, ReasoningContent as RigReasoningContent,
    Text as RigText, UserContent as RigUserContent,
};
use rig_core::one_or_many::OneOrMany;
use rig_core::providers::openai;
use rig_core::streaming::StreamedAssistantContent;

use crate::message::{ContentBlock, Message, Role};
use crate::provider::{Error, LlmProvider};
use crate::request::{CompletionRequest, ReasoningEffort, ToolSchema};
use crate::response::{CompletionResponse, FinishReason, StreamChunk, Usage};

/// OpenAI provider backed by `rig::providers::openai`'s Chat Completions API.
pub struct OpenAiProvider {
    client: openai::CompletionsClient,
}

impl OpenAiProvider {
    /// Build a provider from an explicit API key.
    pub fn from_api_key(api_key: impl AsRef<str>) -> Result<Self, Error> {
        let client = openai::Client::new(api_key.as_ref())
            .map_err(|e| Error::InvalidRequest(format!("openai client init: {e}")))?
            .completions_api();
        Ok(Self { client })
    }

    /// Build a provider from `OPENAI_API_KEY` (and optionally `OPENAI_BASE_URL`).
    pub fn from_env() -> Result<Self, Error> {
        let responses_client = openai::Client::from_env()
            .map_err(|e| Error::InvalidRequest(format!("openai client init: {e}")))?;
        Ok(Self {
            client: responses_client.completions_api(),
        })
    }

    fn build_model(
        &self,
        model: &str,
    ) -> <openai::CompletionsClient as CompletionClient>::CompletionModel {
        self.client.completion_model(model)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn supports_caching(&self) -> bool {
        // OpenAI caches automatically for prompts ≥1024 tokens; we don't
        // honor user-provided breakpoints.
        false
    }

    fn supports_reasoning(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, Error> {
        let model_id = req.model.clone();
        let rig_req = to_rig_request(req)?;
        let model = self.build_model(&model_id);
        let resp = model
            .completion(rig_req)
            .await
            .map_err(|e| classify_completion_error("openai", e))?;

        let content = from_rig_assistant_choice(&resp.choice);
        Ok(CompletionResponse {
            model: model_id,
            content,
            usage: from_rig_usage(&resp.usage),
            // rig doesn't surface finish_reason on its CompletionResponse —
            // map to Stop unconditionally. Length/Length-vs-Tool detection
            // can come from the raw_response when we plumb that through.
            finish_reason: FinishReason::Stop,
        })
    }

    fn stream(
        &self,
        req: CompletionRequest,
    ) -> BoxStream<'static, Result<StreamChunk, Error>> {
        let model_id = req.model.clone();
        let model = self.build_model(&model_id);

        let rig_req = match to_rig_request(req) {
            Ok(r) => r,
            Err(e) => return Box::pin(futures::stream::once(async move { Err(e) })),
        };

        let stream = async_stream::stream! {
            let mut s = match model.stream(rig_req).await {
                Ok(s) => s,
                Err(e) => {
                    yield Err(classify_completion_error("openai", e));
                    return;
                }
            };

            let mut active_tool_call: Option<String> = None;

            while let Some(item) = StreamExt::next(&mut s).await {
                match item {
                    Err(e) => {
                        yield Err(classify_completion_error("openai", e));
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
                            if active_tool_call.as_deref() != Some(&internal_call_id) {
                                yield Ok(StreamChunk::ToolUseStart {
                                    id: tool_call.id.clone(),
                                    name: tool_call.function.name.clone(),
                                });
                            }
                            yield Ok(StreamChunk::ToolUseDelta {
                                partial_json: tool_call.function.arguments.to_string(),
                            });
                            yield Ok(StreamChunk::ToolUseEnd);
                            active_tool_call = None;
                        }
                        StreamedAssistantContent::ToolCallDelta { id, content, .. } => {
                            use rig_core::streaming::ToolCallDeltaContent;
                            match content {
                                ToolCallDeltaContent::Name(name) => {
                                    yield Ok(StreamChunk::ToolUseStart { id, name });
                                }
                                ToolCallDeltaContent::Delta(partial_json) => {
                                    yield Ok(StreamChunk::ToolUseDelta { partial_json });
                                }
                            }
                        }
                        StreamedAssistantContent::Final(_raw) => {
                            // Usage is captured from the consumed stream after
                            // the loop via `s.usage()`/`s.response`.
                        }
                    },
                }
            }

            let _ = active_tool_call;
            // Aggregate final usage from the consumed stream, if available.
            use rig_core::completion::GetTokenUsage;
            if let Some(rig_usage) = s.response.as_ref().and_then(|r| r.token_usage()) {
                yield Ok(StreamChunk::Usage(from_rig_usage(&rig_usage)));
            }
            yield Ok(StreamChunk::Done { finish_reason: FinishReason::Stop });
        };

        Box::pin(stream)
    }
}

// --- Conversions --------------------------------------------------------

fn to_rig_request(req: CompletionRequest) -> Result<RigCompletionRequest, Error> {
    let CompletionRequest {
        model,
        messages,
        tools,
        cache_breakpoints: _,
        reasoning_effort,
        max_tokens,
        temperature,
    } = req;

    let mut chat_history: Vec<RigMessage> = Vec::with_capacity(messages.len());
    for m in messages {
        chat_history.push(to_rig_message(m)?);
    }
    let chat_history = OneOrMany::many(chat_history)
        .map_err(|_| Error::InvalidRequest("messages must not be empty".into()))?;

    let tools = tools
        .into_iter()
        .map(to_rig_tool_def)
        .collect::<Vec<_>>();

    let additional_params = reasoning_effort.and_then(to_openai_reasoning_param);

    Ok(RigCompletionRequest {
        model: Some(model),
        preamble: None,
        chat_history,
        documents: Vec::new(),
        tools,
        temperature: temperature.map(|t| t as f64),
        max_tokens: Some(max_tokens as u64),
        tool_choice: None,
        additional_params,
        output_schema: None,
    })
}

fn to_rig_message(m: Message) -> Result<RigMessage, Error> {
    match m.role {
        Role::System => {
            let text = collect_text(&m.content);
            Ok(RigMessage::System { content: text })
        }
        Role::User | Role::Tool => {
            let mut items = Vec::new();
            for block in m.content {
                match block {
                    ContentBlock::Text { text } => {
                        items.push(RigUserContent::Text(RigText { text }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                        trustworthy: _,
                    } => {
                        use rig_core::completion::message::{Text, ToolResult, ToolResultContent};
                        let body = if is_error {
                            format!("[error] {content}")
                        } else {
                            content
                        };
                        items.push(RigUserContent::ToolResult(ToolResult {
                            id: tool_use_id,
                            call_id: None,
                            content: OneOrMany::one(ToolResultContent::Text(Text { text: body })),
                        }));
                    }
                    ContentBlock::ToolUse { .. } | ContentBlock::Thinking { .. } => {
                        return Err(Error::InvalidRequest(
                            "tool_use/thinking blocks not valid on a user/tool message".into(),
                        ));
                    }
                }
            }
            let content = OneOrMany::many(items).map_err(|_| {
                Error::InvalidRequest("user/tool message must contain at least one block".into())
            })?;
            Ok(RigMessage::User { content })
        }
        Role::Assistant => {
            let mut items = Vec::new();
            for block in m.content {
                match block {
                    ContentBlock::Text { text } => {
                        items.push(RigAssistantContent::Text(RigText { text }));
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        use rig_core::completion::message::{ToolCall, ToolFunction};
                        items.push(RigAssistantContent::ToolCall(ToolCall::new(
                            id,
                            ToolFunction::new(name, input),
                        )));
                    }
                    ContentBlock::Thinking { thinking, signature } => {
                        items.push(RigAssistantContent::Reasoning(
                            rig_core::completion::message::Reasoning::new_with_signature(
                                &thinking, signature,
                            ),
                        ));
                    }
                    ContentBlock::ToolResult { .. } => {
                        return Err(Error::InvalidRequest(
                            "tool_result not valid on an assistant message".into(),
                        ));
                    }
                }
            }
            let content = OneOrMany::many(items).map_err(|_| {
                Error::InvalidRequest("assistant message must contain at least one block".into())
            })?;
            Ok(RigMessage::Assistant { id: None, content })
        }
    }
}

fn to_rig_tool_def(t: ToolSchema) -> RigToolDefinition {
    RigToolDefinition {
        name: t.name,
        description: t.description,
        parameters: t.parameters,
    }
}

fn to_openai_reasoning_param(eff: ReasoningEffort) -> Option<serde_json::Value> {
    let s = match eff {
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        // Anthropic-specific budget; map to high for o-series.
        ReasoningEffort::Budget(_) => "high",
    };
    Some(serde_json::json!({ "reasoning_effort": s }))
}

fn from_rig_assistant_choice(choice: &OneOrMany<RigAssistantContent>) -> Vec<ContentBlock> {
    choice
        .iter()
        .filter_map(|c| match c {
            RigAssistantContent::Text(t) => Some(ContentBlock::Text {
                text: t.text.clone(),
            }),
            RigAssistantContent::ToolCall(call) => Some(ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.function.name.clone(),
                input: call.function.arguments.clone(),
            }),
            RigAssistantContent::Reasoning(r) => {
                let combined = r
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        RigReasoningContent::Text { text, signature } => {
                            Some((text.clone(), signature.clone()))
                        }
                        _ => None,
                    })
                    .next();
                combined.map(|(thinking, signature)| ContentBlock::Thinking {
                    thinking,
                    signature,
                })
            }
            RigAssistantContent::Image(_) => None,
        })
        .collect()
}

fn from_rig_usage(u: &rig_core::completion::Usage) -> Usage {
    Usage {
        input_tokens: u.input_tokens as u32,
        output_tokens: u.output_tokens as u32,
        cache_read_input_tokens: u.cached_input_tokens as u32,
        cache_creation_input_tokens: u.cache_creation_input_tokens as u32,
        reasoning_tokens: u.reasoning_tokens as u32,
    }
}

fn collect_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn classify_completion_error(
    provider: &'static str,
    e: rig_core::completion::CompletionError,
) -> Error {
    use rig_core::completion::CompletionError;
    let s = e.to_string();
    match e {
        CompletionError::HttpError(_) => Error::Transport(s),
        CompletionError::ProviderError(_) => {
            let status = if s.contains("429") {
                429
            } else if s.contains("500") || s.contains("502") || s.contains("503") {
                500
            } else if s.contains("context") || s.contains("token") && s.contains("limit") {
                return Error::ContextLengthExceeded {
                    model: String::new(),
                };
            } else {
                400
            };
            Error::Provider {
                provider,
                status,
                message: s,
                retry_after: None,
            }
        }
        CompletionError::JsonError(_) => Error::Serialization(s),
        CompletionError::RequestError(_) => Error::Transport(s),
        _ => Error::Provider {
            provider,
            status: 500,
            message: s,
            retry_after: None,
        },
    }
}

// Silence unused-import for futures Stream/Pin in case stream macro paths shift.
#[allow(dead_code)]
fn _retry_after_marker() -> Duration {
    Duration::from_secs(1)
}

#[allow(dead_code)]
type _StreamCompat<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
