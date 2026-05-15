use std::sync::Arc;

use async_trait::async_trait;
use ravn_persistence::Db;
use tokio_util::sync::CancellationToken;

use crate::tool::Permission;

/// Decision returned by an [`Approver`]. `AllowAndRemember` adds a
/// pattern to the allowlist so future calls bypass the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
    AllowAndRemember,
}

/// Approval strategy — concrete implementations vary by frontend:
/// * `ravn-cli` shows an inline TUI modal (Phase 1.9, D7).
/// * Tests use `MockApprover` (always-allow / always-deny).
/// * Future MCP-server mode uses a static allowlist.
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(
        &self,
        tool: &str,
        args: &serde_json::Value,
        permission: Permission,
    ) -> ApprovalDecision;
}

/// Approver that always allows. Suitable for tests and read-only tool
/// suites where no Write/Exec tools are registered.
pub struct AllowAll;

#[async_trait]
impl Approver for AllowAll {
    async fn approve(
        &self,
        _tool: &str,
        _args: &serde_json::Value,
        _permission: Permission,
    ) -> ApprovalDecision {
        ApprovalDecision::Allow
    }
}

/// Approver that always denies. Useful when running an agent in a
/// hands-off mode that must not perform any Write/Exec actions.
pub struct DenyAll;

#[async_trait]
impl Approver for DenyAll {
    async fn approve(
        &self,
        _tool: &str,
        _args: &serde_json::Value,
        _permission: Permission,
    ) -> ApprovalDecision {
        ApprovalDecision::Deny
    }
}

/// Per-invocation environment handed to every [`crate::Tool::invoke`].
///
/// Holds the DB handle for persistence access, the session correlation
/// IDs, the cancellation token plumbed from the agent loop, and the
/// approver to consult before performing Write/Exec actions.
#[derive(Clone)]
pub struct ToolContext {
    pub db: Db,
    pub session_id: String,
    pub trace_id: String,
    pub cancel: CancellationToken,
    pub approver: Arc<dyn Approver>,
}

impl ToolContext {
    pub fn new(
        db: Db,
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        approver: Arc<dyn Approver>,
    ) -> Self {
        Self {
            db,
            session_id: session_id.into(),
            trace_id: trace_id.into(),
            cancel: CancellationToken::new(),
            approver,
        }
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}
