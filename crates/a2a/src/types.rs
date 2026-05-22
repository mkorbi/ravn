//! A2A (Agent2Agent) protocol types — hand-rolled per the spec.
//!
//! Only the subset ravn needs: Agent Card, Message/Part, Task/TaskStatus/
//! TaskState, Artifact, and the JSON-RPC 2.0 envelope. Field names follow the
//! spec's camelCase via `rename_all`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Agent Card
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub protocol_version: String,
    pub name: String,
    pub description: String,
    /// Base URL of this agent's A2A JSON-RPC endpoint.
    pub url: String,
    pub version: String,
    pub capabilities: AgentCapabilities,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
    /// OpenAPI-style security schemes (kept as raw JSON — we only emit them).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub security_schemes: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub push_notifications: bool,
    #[serde(default)]
    pub state_transition_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

// ---------------------------------------------------------------------------
// Message / Part
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Agent,
}

/// A content part. The spec discriminates on `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    Text { text: String },
    File { file: Value },
    Data { data: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default = "kind_message")]
    pub kind: String,
}

fn kind_message() -> String {
    "message".to_string()
}

impl Message {
    /// Build an `agent`-role text message (what ravn replies with).
    pub fn agent_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::Agent,
            parts: vec![Part::Text { text: text.into() }],
            message_id: uuid::Uuid::new_v4().to_string(),
            task_id: None,
            context_id: None,
            kind: kind_message(),
        }
    }

    /// Concatenate all text parts (ignores file/data parts).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for p in &self.parts {
            if let Part::Text { text } = p {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    Submitted,
    Working,
    InputRequired,
    Completed,
    Canceled,
    Failed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

impl TaskStatus {
    pub fn new(state: TaskState) -> Self {
        Self {
            state,
            message: None,
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub parts: Vec<Part>,
}

impl Artifact {
    pub fn text(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            artifact_id: uuid::Uuid::new_v4().to_string(),
            name: Some(name.into()),
            parts: vec![Part::Text { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    #[serde(default = "kind_task")]
    pub kind: String,
}

fn kind_task() -> String {
    "task".to_string()
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 envelope + method params
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Value, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            error: None,
        }
    }

    pub fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// JSON-RPC error codes used by the A2A server.
pub mod error_code {
    pub const PARSE: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;
    /// A2A: task not found.
    pub const TASK_NOT_FOUND: i64 = -32001;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSendParams {
    pub message: Message,
    #[serde(default)]
    pub configuration: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskIdParams {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_uses_kind_discriminator() {
        let p = Part::Text {
            text: "hi".into(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "text");
        assert_eq!(v["text"], "hi");
    }

    #[test]
    fn task_state_is_kebab_case() {
        assert_eq!(
            serde_json::to_value(TaskState::InputRequired).unwrap(),
            "input-required"
        );
        assert_eq!(serde_json::to_value(TaskState::Completed).unwrap(), "completed");
    }

    #[test]
    fn message_round_trips_and_extracts_text() {
        let json = r#"{"role":"user","parts":[{"kind":"text","text":"hello"}],"messageId":"m1","kind":"message"}"#;
        let m: Message = serde_json::from_str(json).unwrap();
        assert_eq!(m.role, Role::User);
        assert_eq!(m.text(), "hello");
        // re-serialize → camelCase messageId present
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["messageId"], "m1");
    }

    #[test]
    fn agent_card_serializes_camelcase() {
        let card = AgentCard {
            protocol_version: "0.3.0".into(),
            name: "ravn".into(),
            description: "test".into(),
            url: "http://localhost/".into(),
            version: "0.0.1".into(),
            capabilities: AgentCapabilities::default(),
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
            skills: vec![],
            security_schemes: BTreeMap::new(),
            security: vec![],
        };
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["protocolVersion"], "0.3.0");
        assert_eq!(v["defaultInputModes"][0], "text/plain");
    }
}
