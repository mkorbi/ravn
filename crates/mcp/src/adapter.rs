//! Adapter that exposes one remote MCP tool as a [`ravn_tools::Tool`].
//!
//! Each adapter holds a clone of the rmcp `Peer<RoleClient>` for its
//! source server plus the namespaced tool name (`<server>__<tool>`).
//! `invoke()` packages the user-supplied JSON args into
//! `CallToolRequestParams` and round-trips through the MCP server.

use async_trait::async_trait;
use ravn_tools::{Permission, Tool, ToolContext, ToolError, ToolOutput};
use rmcp::model::{CallToolRequestParams, Content, JsonObject, RawContent};
use rmcp::service::{Peer, RoleClient};

pub struct McpToolAdapter {
    /// Leaked static string `<server>__<tool>` so we satisfy
    /// `Tool::name(&self) -> &'static str`. One leak per registered
    /// tool, bounded by the number of tools the user has configured.
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) permission: Permission,
    pub(crate) schema: serde_json::Value,
    /// The unprefixed name as the MCP server knows it.
    pub(crate) remote_name: String,
    /// MCP server peer (cheap to clone — it's an Arc inside).
    pub(crate) peer: Peer<RoleClient>,
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        self.description
    }
    fn permission(&self) -> Permission {
        self.permission
    }
    fn schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    async fn invoke(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let arguments: Option<JsonObject> = match args {
            serde_json::Value::Null => None,
            serde_json::Value::Object(o) => Some(o),
            other => {
                return Err(ToolError::InvalidArgs(format!(
                    "MCP tools require an object args payload; got {other}"
                )));
            }
        };

        let mut params = CallToolRequestParams::new(self.remote_name.clone());
        params.arguments = arguments;
        let result = self
            .peer
            .call_tool(params)
            .await
            .map_err(|e| ToolError::Internal(format!("call_tool({}): {e}", self.remote_name)))?;

        let body = render_content(&result.content);
        let is_error = result.is_error.unwrap_or(false);
        if is_error {
            return Ok(ToolOutput::error(body));
        }
        // MCP results from external servers are always untrusted —
        // wrap them via trustworthy=false (Phase 1.10 handling kicks in
        // when ContentBlock::ToolResult goes back to the model).
        Ok(ToolOutput::untrusted(body))
    }
}

fn render_content(items: &[Content]) -> String {
    let mut buf = String::new();
    for c in items {
        match &c.raw {
            RawContent::Text(t) => {
                buf.push_str(&t.text);
                buf.push('\n');
            }
            RawContent::Image(_) => buf.push_str("[image content omitted]\n"),
            RawContent::Audio(_) => buf.push_str("[audio content omitted]\n"),
            RawContent::Resource(_) => buf.push_str("[resource link omitted]\n"),
            RawContent::ResourceLink(_) => buf.push_str("[resource link omitted]\n"),
        }
    }
    if buf.is_empty() {
        "[empty MCP result]".to_string()
    } else {
        buf.trim_end().to_string()
    }
}
