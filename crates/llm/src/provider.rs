use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::request::CompletionRequest;
use crate::response::{CompletionResponse, StreamChunk};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider identifier, e.g. `"anthropic"`, `"openai"`. Used in tracing
    /// spans and cost-tracking attribution.
    fn name(&self) -> &'static str;

    /// True if the provider honors `CompletionRequest::cache_breakpoints`
    /// (Anthropic). OpenAI caches automatically without explicit markers.
    fn supports_caching(&self) -> bool;

    /// True if the provider exposes `reasoning_effort` (o-series) or
    /// `thinking.budget_tokens` (Anthropic Extended Thinking).
    fn supports_reasoning(&self) -> bool;

    /// Non-streaming completion. Adapters MUST populate `Usage` from the
    /// provider response.
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, Error>;

    /// Streaming completion. The returned stream MUST emit `StreamChunk::Done`
    /// as its terminal item under normal completion. Errors mid-stream are
    /// surfaced as `Err(Error)` items.
    fn stream(
        &self,
        req: CompletionRequest,
    ) -> BoxStream<'static, Result<StreamChunk, Error>>;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Provider returned a non-success HTTP status. `retry_after` is set when
    /// the response indicates rate-limit backoff.
    #[error("provider {provider} returned {status}: {message}")]
    Provider {
        provider: &'static str,
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },

    /// Request payload was rejected before reaching the provider (e.g. too
    /// many cache breakpoints, malformed tool schema).
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// Conversation exceeds the model's context window.
    #[error("context length exceeded for model {model}")]
    ContextLengthExceeded { model: String },

    /// Failure encoding the request or decoding the response.
    #[error("serialization: {0}")]
    Serialization(String),

    /// Network / transport-level failure (timeouts, DNS, TLS).
    #[error("transport: {0}")]
    Transport(String),

    /// The caller's `CancellationToken` fired.
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    /// `true` if the caller should retry the same request after a backoff.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Error::Provider { status: 429 | 500..=599, .. } | Error::Transport(_)
        )
    }

    /// Backoff suggested by the provider's `Retry-After` header, if any.
    pub fn retry_after(&self) -> Option<Duration> {
        if let Error::Provider { retry_after, .. } = self {
            *retry_after
        } else {
            None
        }
    }
}
