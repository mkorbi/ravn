use serde::{Deserialize, Serialize};

use crate::message::ContentBlock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub usage: Usage,
    pub finish_reason: FinishReason,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens served from the provider's prompt cache (10× cheaper on Anthropic,
    /// 50% cheaper on OpenAI).
    pub cache_read_input_tokens: u32,
    /// Tokens written to the cache on this call (Anthropic-only billing line).
    pub cache_creation_input_tokens: u32,
    /// Hidden reasoning tokens from o-series / Extended Thinking. Counted as
    /// output tokens by providers but tracked separately for cost analysis.
    pub reasoning_tokens: u32,
}

impl Usage {
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Natural stop, model emitted end-of-turn.
    Stop,
    /// Hit `max_tokens`.
    Length,
    /// Model emitted tool calls and expects a tool result next.
    ToolUse,
    /// Provider content filter or safety policy intervened.
    ContentFilter,
    /// Other / unknown reason returned by provider.
    Other,
}

/// Incremental update in a streaming completion. Adapters translate provider
/// SSE/JSONL formats into this enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamChunk {
    /// Visible text token(s).
    TextDelta(String),
    /// Anthropic Extended Thinking delta — not shown to end-users by default
    /// but must be retained across turns to preserve cache coherence.
    ThinkingDelta(String),
    /// Start of a tool-use block. `input` is empty; partial deltas follow.
    ToolUseStart { id: String, name: String },
    /// Partial JSON for the tool-use input — concatenate to reconstruct.
    ToolUseDelta { partial_json: String },
    /// Tool-use block finished.
    ToolUseEnd,
    /// Final usage stats — emitted once near end-of-stream.
    Usage(Usage),
    /// Terminal chunk.
    Done { finish_reason: FinishReason },
}
