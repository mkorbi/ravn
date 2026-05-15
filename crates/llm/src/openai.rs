//! OpenAI adapter — wraps `rig::providers::openai` (D1).
//!
//! Phase 0.4 scope: text + reasoning + basic tool-call passthrough.
//! Cache breakpoints are accepted but ignored — OpenAI manages its prompt
//! cache automatically.

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use rig_core::client::{CompletionClient, ProviderClient};
use rig_core::completion::CompletionModel as _;
use rig_core::completion::message::ReasoningContent as RigReasoningContent;
use rig_core::providers::openai;
use rig_core::streaming::StreamedAssistantContent;

use crate::provider::{Error, LlmProvider};
use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, FinishReason, StreamChunk};
use crate::rig_bridge::{
    classify_completion_error, from_rig_assistant_choice, from_rig_usage, openai_reasoning_param,
    to_rig_request,
};

/// OpenAI provider backed by `rig::providers::openai::CompletionsClient`
/// (Chat Completions API).
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

    fn model(
        &self,
        name: &str,
    ) -> <openai::CompletionsClient as CompletionClient>::CompletionModel {
        self.client.completion_model(name)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn supports_caching(&self) -> bool {
        false
    }

    fn supports_reasoning(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, Error> {
        let model_id = req.model.clone();
        let reasoning_param = req.reasoning_effort.map(openai_reasoning_param);
        let mut rig_req = to_rig_request(req)?;
        rig_req.additional_params = reasoning_param;

        let model = self.model(&model_id);
        let resp = model
            .completion(rig_req)
            .await
            .map_err(|e| classify_completion_error("openai", e))?;

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
        let reasoning_param = req.reasoning_effort.map(openai_reasoning_param);
        let model = self.model(&model_id);

        let mut rig_req = match to_rig_request(req) {
            Ok(r) => r,
            Err(e) => return Box::pin(futures::stream::once(async move { Err(e) })),
        };
        rig_req.additional_params = reasoning_param;

        Box::pin(async_stream::stream! {
            let mut s = match model.stream(rig_req).await {
                Ok(s) => s,
                Err(e) => {
                    yield Err(classify_completion_error("openai", e));
                    return;
                }
            };

            // See `anthropic.rs` for the rationale — providers can emit
            // both ToolCallDelta and a final ToolCall for the same tool;
            // we must not double-emit Start/Delta/End.
            let mut delta_tool: Option<String> = None;

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
                            if delta_tool.as_deref() == Some(&internal_call_id) {
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
