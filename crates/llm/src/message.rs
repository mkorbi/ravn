use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },

    /// Assistant requests a tool invocation.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// Result of a previous tool invocation, attached to a user-role message.
    /// `trustworthy = false` triggers prompt-injection-safe rendering in adapters.
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        trustworthy: bool,
    },

    /// Anthropic Extended Thinking block — must be preserved across turns
    /// to keep the prompt cache valid. The `signature` is provider-opaque.
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
}
