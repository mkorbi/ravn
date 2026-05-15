//! LLM-Provider-Abstraktion (D1).
//!
//! Defines the [`LlmProvider`] trait and the request/response types that all
//! provider adapters speak. Adapters live in submodules (`openai`, `anthropic`,
//! ...) and translate between this crate's vocabulary and the provider's wire
//! format.
//!
//! See `project.md` §1.1 for the architectural rationale and `PLAN.md` D1 for
//! the framework decision (`rig-core` for provider backends, but the ReAct
//! loop and trait surface live here, under our control).

pub mod message;
pub mod openai;
pub mod provider;
pub mod request;
pub mod response;

pub use message::{ContentBlock, Message, Role};
pub use provider::{Error, LlmProvider};
pub use request::{
    CacheBreakpoint, CachePosition, CacheTtl, CompletionRequest, ReasoningEffort, ToolSchema,
};
pub use response::{CompletionResponse, FinishReason, StreamChunk, Usage};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_helpers_produce_text_block() {
        let m = Message::user("hello");
        assert_eq!(m.role, Role::User);
        assert!(matches!(&m.content[0], ContentBlock::Text { text } if text == "hello"));
    }

    #[test]
    fn request_roundtrips_through_serde() {
        let req = CompletionRequest::new(
            "claude-opus-4-7",
            vec![Message::system("be helpful"), Message::user("hi")],
            1024,
        );
        let json = serde_json::to_string(&req).unwrap();
        let back: CompletionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, "claude-opus-4-7");
        assert_eq!(back.messages.len(), 2);
        assert_eq!(back.max_tokens, 1024);
    }

    #[test]
    fn tool_use_content_block_roundtrips() {
        let block = ContentBlock::ToolUse {
            id: "toolu_1".into(),
            name: "file_read".into(),
            input: serde_json::json!({"path": "/etc/hosts"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"tool_use""#));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ContentBlock::ToolUse { name, .. } if name == "file_read"));
    }

    #[test]
    fn provider_error_classifies_retries() {
        let e = Error::Provider {
            provider: "anthropic",
            status: 429,
            message: "rate limit".into(),
            retry_after: Some(std::time::Duration::from_secs(2)),
        };
        assert!(e.is_retryable());
        assert_eq!(e.retry_after(), Some(std::time::Duration::from_secs(2)));

        let e = Error::InvalidRequest("bad schema".into());
        assert!(!e.is_retryable());
    }
}
