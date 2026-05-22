//! Per-job approver for unattended heartbeat runs (Phase 4.10).

use std::collections::HashSet;

use async_trait::async_trait;
use ravn_tools::{ApprovalDecision, Approver, Permission};

/// Auto-approves only the tools in a job's `allow_tools`, denying everything
/// else. The agent loop consults the approver **only** for Write/Exec tools
/// (Read tools bypass it), so any tool that reaches us is gated: allowed iff
/// it is in the allowlist, otherwise denied and logged.
pub struct AllowlistApprover {
    allow: HashSet<String>,
}

impl AllowlistApprover {
    pub fn new(allow: HashSet<String>) -> Self {
        Self { allow }
    }
}

#[async_trait]
impl Approver for AllowlistApprover {
    async fn approve(
        &self,
        tool: &str,
        _args: &serde_json::Value,
        permission: Permission,
    ) -> ApprovalDecision {
        if self.allow.contains(tool) {
            ApprovalDecision::Allow
        } else {
            tracing::warn!(
                tool,
                ?permission,
                "heartbeat: tool not in job allow_tools — denied"
            );
            ApprovalDecision::Deny
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_listed_denies_others() {
        let approver = AllowlistApprover::new(["calendar__create_event".to_string()].into());
        assert_eq!(
            approver
                .approve(
                    "calendar__create_event",
                    &serde_json::json!({}),
                    Permission::Write
                )
                .await,
            ApprovalDecision::Allow
        );
        assert_eq!(
            approver
                .approve("shell", &serde_json::json!({}), Permission::Exec)
                .await,
            ApprovalDecision::Deny
        );
    }
}
