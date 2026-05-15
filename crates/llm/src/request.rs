use serde::{Deserialize, Serialize};

use crate::message::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSchema>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cache_breakpoints: Vec<CacheBreakpoint>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,

    pub max_tokens: u32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

impl CompletionRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>, max_tokens: u32) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
            cache_breakpoints: Vec::new(),
            reasoning_effort: None,
            max_tokens,
            temperature: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input parameters. Generated via `schemars`
    /// in higher layers; raw `serde_json::Value` here to avoid leaking
    /// `schemars` into the LLM crate's public API.
    pub parameters: serde_json::Value,
}

/// Cache breakpoint placed at a specific position in the request to enable
/// provider-side prompt caching. Anthropic supports up to 4 breakpoints with
/// `cache_control: { type: "ephemeral" }`. OpenAI caches automatically and
/// ignores these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheBreakpoint {
    pub position: CachePosition,
    pub ttl: CacheTtl,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePosition {
    /// After all tool definitions.
    EndOfTools,
    /// After the system prompt.
    EndOfSystem,
    /// After the message at this index in `CompletionRequest::messages`.
    MessageIndex(usize),
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtl {
    #[default]
    FiveMinutes,
    /// Anthropic only, via `anthropic-beta: extended-cache-ttl-2025-04-11` header.
    OneHour,
}

/// User-facing reasoning intensity. Adapters map this to provider format:
/// - OpenAI o-series: `Low|Medium|High` → `reasoning_effort` string.
/// - Anthropic Extended Thinking: mapped to `thinking.budget_tokens`.
/// - DeepSeek R1: ignored (always full reasoning).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    /// Explicit Anthropic thinking budget in tokens.
    Budget(u32),
}
