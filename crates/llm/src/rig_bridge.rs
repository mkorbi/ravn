//! Shared conversion logic between our [`crate`] types and `rig-core`'s.
//!
//! All adapters that wrap a rig provider should use these helpers to keep the
//! translation surface in one place.

use rig_core::completion::message::{
    AssistantContent as RigAssistantContent, DocumentSourceKind, Image as RigImage, ImageMediaType,
    ReasoningContent as RigReasoningContent, Text as RigText, ToolCall, ToolFunction, ToolResult,
    ToolResultContent, UserContent as RigUserContent,
};
use rig_core::completion::{
    CompletionRequest as RigCompletionRequest, Message as RigMessage,
    ToolDefinition as RigToolDefinition,
};
use rig_core::one_or_many::OneOrMany;

use crate::message::{ContentBlock, ImageContent, Message, Role};
use crate::provider::Error;
use crate::request::{CompletionRequest, ReasoningEffort, ToolSchema};
use crate::response::Usage;

pub fn to_rig_request(req: CompletionRequest) -> Result<RigCompletionRequest, Error> {
    let CompletionRequest {
        model,
        messages,
        tools,
        cache_breakpoints: _,
        reasoning_effort: _,
        max_tokens,
        temperature,
    } = req;

    let mut chat_history: Vec<RigMessage> = Vec::with_capacity(messages.len());
    for m in messages {
        chat_history.push(to_rig_message(m)?);
    }
    let chat_history = OneOrMany::many(chat_history)
        .map_err(|_| Error::InvalidRequest("messages must not be empty".into()))?;

    let tools = tools.into_iter().map(to_rig_tool_def).collect::<Vec<_>>();

    Ok(RigCompletionRequest {
        model: Some(model),
        preamble: None,
        chat_history,
        documents: Vec::new(),
        tools,
        temperature: temperature.map(|t| t as f64),
        max_tokens: Some(max_tokens as u64),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    })
}

pub fn to_rig_message(m: Message) -> Result<RigMessage, Error> {
    match m.role {
        Role::System => Ok(RigMessage::System {
            content: collect_text(&m.content),
        }),
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
                        trustworthy,
                    } => {
                        // Phase 1.10: wrap outputs from untrusted sources
                        // (web_fetch, untrusted files, MCP servers of unknown
                        // origin) so the model is hinted to treat the contents
                        // as data, not instructions. Internal-error strings
                        // (our denial / wrapper messages) keep the un-wrapped
                        // form — they're trustworthy by construction.
                        let body = match (is_error, trustworthy) {
                            (true, _) => format!("[error] {content}"),
                            (false, true) => content,
                            (false, false) => format!(
                                "<tool_result trustworthy=\"false\">\n{content}\n</tool_result>"
                            ),
                        };
                        items.push(RigUserContent::ToolResult(ToolResult {
                            id: tool_use_id,
                            call_id: None,
                            content: OneOrMany::one(ToolResultContent::Text(RigText {
                                text: body,
                            })),
                        }));
                    }
                    ContentBlock::Image { image } => {
                        items.push(RigUserContent::Image(to_rig_image(image)));
                    }
                    ContentBlock::ToolUse { .. } | ContentBlock::Thinking { .. } => {
                        return Err(Error::InvalidRequest(
                            "tool_use/thinking blocks not valid on user/tool message".into(),
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
                    ContentBlock::Image { .. } => {
                        return Err(Error::InvalidRequest(
                            "image not valid on assistant message".into(),
                        ));
                    }
                    ContentBlock::ToolResult { .. } => {
                        return Err(Error::InvalidRequest(
                            "tool_result not valid on assistant message".into(),
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

pub fn to_rig_tool_def(t: ToolSchema) -> RigToolDefinition {
    RigToolDefinition {
        name: t.name,
        description: t.description,
        parameters: t.parameters,
    }
}

pub fn from_rig_assistant_choice(choice: &OneOrMany<RigAssistantContent>) -> Vec<ContentBlock> {
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
            RigAssistantContent::Reasoning(r) => r.content.iter().find_map(|b| match b {
                RigReasoningContent::Text { text, signature } => Some(ContentBlock::Thinking {
                    thinking: text.clone(),
                    signature: signature.clone(),
                }),
                _ => None,
            }),
            RigAssistantContent::Image(_) => None,
        })
        .collect()
}

pub fn from_rig_usage(u: &rig_core::completion::Usage) -> Usage {
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

/// Map our [`ImageContent`] to rig's `Image` (Phase 5.6).
fn to_rig_image(image: ImageContent) -> RigImage {
    match image {
        ImageContent::Url { url } => RigImage {
            data: DocumentSourceKind::Url(url),
            media_type: None,
            detail: None,
            additional_params: None,
        },
        ImageContent::Base64 { media_type, data } => RigImage {
            data: DocumentSourceKind::Base64(data),
            media_type: rig_media_type(&media_type),
            detail: None,
            additional_params: None,
        },
    }
}

fn rig_media_type(mime: &str) -> Option<ImageMediaType> {
    match mime {
        "image/png" => Some(ImageMediaType::PNG),
        "image/jpeg" => Some(ImageMediaType::JPEG),
        "image/gif" => Some(ImageMediaType::GIF),
        "image/webp" => Some(ImageMediaType::WEBP),
        _ => None,
    }
}

pub fn openai_reasoning_param(eff: ReasoningEffort) -> serde_json::Value {
    let s = match eff {
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Budget(_) => "high",
    };
    serde_json::json!({ "reasoning_effort": s })
}

/// Map a `ReasoningEffort` to an Anthropic Extended-Thinking `thinking` block,
/// injected via `additional_params`.
pub fn anthropic_thinking_param(eff: ReasoningEffort) -> serde_json::Value {
    let budget = match eff {
        ReasoningEffort::Low => 4_096,
        ReasoningEffort::Medium => 8_192,
        ReasoningEffort::High => 16_384,
        ReasoningEffort::Budget(n) => n,
    };
    serde_json::json!({
        "thinking": {
            "type": "enabled",
            "budget_tokens": budget,
        }
    })
}

pub fn classify_completion_error(
    provider: &'static str,
    e: rig_core::completion::CompletionError,
) -> Error {
    use rig_core::completion::CompletionError;
    let s = e.to_string();
    match e {
        CompletionError::HttpError(_) => Error::Transport(s),
        CompletionError::ProviderError(_) => {
            if s.contains("context") || (s.contains("token") && s.contains("limit")) {
                return Error::ContextLengthExceeded {
                    model: String::new(),
                };
            }
            let status = if s.contains("429") {
                429
            } else if s.contains("500")
                || s.contains("502")
                || s.contains("503")
                || s.contains("504")
            {
                500
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_image_maps_to_rig_image() {
        let m = Message::user_multimodal(
            Some("hi".into()),
            vec![ImageContent::Url {
                url: "https://x/y.png".into(),
            }],
        );
        match to_rig_message(m).unwrap() {
            RigMessage::User { content } => {
                assert!(content
                    .iter()
                    .any(|c| matches!(c, RigUserContent::Image(_))));
            }
            _ => panic!("expected a user message"),
        }
    }
}
