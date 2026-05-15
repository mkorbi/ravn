//! TUI Approver impl (Phase 1.9, D7).
//!
//! `Agent::run` calls `Approver::approve()` whenever it hits a Write or
//! Exec tool. This impl forwards the request to the UI loop via the same
//! `AppEvent` channel; the UI shows a modal, captures `y`/`n`/`a`/Esc,
//! and sends the decision back via a `oneshot`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ravn_tools::{ApprovalDecision, Approver, Permission};
use tokio::sync::{mpsc, oneshot};

use crate::app::AppEvent;

pub struct ApprovalRequest {
    pub tool: String,
    pub args: serde_json::Value,
    pub permission: Permission,
    /// One-shot channel back to the requesting tool — answered by the
    /// UI when the user resolves the modal.
    pub responder: oneshot::Sender<ApprovalDecision>,
}

pub struct TuiApprover {
    tx: mpsc::Sender<AppEvent>,
    /// In-memory per-session allowlist of tool names that the user has
    /// approved with `a`. Persistence across sessions is a Phase-1
    /// follow-up (would need a new `tool_allowlist` DB table).
    allowlist: Arc<Mutex<HashSet<String>>>,
}

impl TuiApprover {
    pub fn new(tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            tx,
            allowlist: Arc::new(Mutex::new(HashSet::new())),
        }
    }

}

#[async_trait]
impl Approver for TuiApprover {
    async fn approve(
        &self,
        tool: &str,
        args: &serde_json::Value,
        permission: Permission,
    ) -> ApprovalDecision {
        if self.allowlist.lock().unwrap().contains(tool) {
            return ApprovalDecision::Allow;
        }

        let (resp_tx, resp_rx) = oneshot::channel();
        let req = ApprovalRequest {
            tool: tool.to_string(),
            args: args.clone(),
            permission,
            responder: resp_tx,
        };
        if self.tx.send(AppEvent::Approval(req)).await.is_err() {
            return ApprovalDecision::Deny;
        }
        match resp_rx.await {
            Ok(ApprovalDecision::AllowAndRemember) => {
                self.allowlist.lock().unwrap().insert(tool.to_string());
                ApprovalDecision::Allow
            }
            Ok(d) => d,
            Err(_) => ApprovalDecision::Deny,
        }
    }
}
