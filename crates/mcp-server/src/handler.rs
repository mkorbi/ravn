//! `ServerHandler` that exposes ravn's read-only tools over MCP.

use std::collections::HashSet;
use std::sync::Arc;

use ravn_persistence::Db;
use ravn_tools::{AllowAll, ToolContext, ToolRegistry};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorData as McpError, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::RoleServer;

#[derive(Clone)]
pub struct RavnServer {
    pub registry: Arc<ToolRegistry>,
    pub db: Db,
    /// Tool names the client is allowed to list + call (all `Permission::Read`).
    pub exposed: HashSet<String>,
}

impl ServerHandler for RavnServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("ravn", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "ravn personal-assistant tools (read-only): search past sessions, browse skills, \
             and get the current date/time."
                .to_string(),
        );
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools = Vec::new();
        for name in &self.exposed {
            if let Some(t) = self.registry.get(name) {
                let schema = t.schema().as_object().cloned().unwrap_or_default();
                tools.push(Tool::new(
                    name.clone(),
                    t.description().to_string(),
                    Arc::new(schema),
                ));
            }
        }
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        if !self.exposed.contains(name) {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "tool '{name}' is not exposed by this server"
            ))]));
        }
        let Some(tool) = self.registry.get(name) else {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "tool '{name}' not found"
            ))]));
        };
        let args = match request.arguments {
            Some(obj) => serde_json::Value::Object(obj),
            None => serde_json::Value::Null,
        };
        // Only Read tools are ever exposed, so they never consult the approver;
        // AllowAll is correct and the exposure set is the real gate.
        let ctx = ToolContext::new(
            self.db.clone(),
            "mcp-server",
            uuid::Uuid::new_v4().to_string(),
            Arc::new(AllowAll),
        );
        match tool.invoke(args, &ctx).await {
            Ok(out) if out.is_error => {
                Ok(CallToolResult::error(vec![Content::text(out.content)]))
            }
            Ok(out) => Ok(CallToolResult::success(vec![Content::text(out.content)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "tool error: {e}"
            ))])),
        }
    }
}
