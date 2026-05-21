//! TUI Approver impl (Phase 1.9, D7) with persistent allowlist (Phase 2.11, D13).
//!
//! `Agent::run` calls `Approver::approve()` whenever it hits a Write or
//! Exec tool. This impl:
//!
//! 1. Looks up the tool name in the in-memory allowlist (populated from
//!    the `tool_allowlist` DB table at startup + every successful
//!    `AllowAndRemember` decision).
//! 2. If miss: forwards the request to the UI loop via the
//!    `AppEvent::Approval` channel, awaits the `y`/`n`/`a`/Esc decision
//!    via a oneshot, and returns it.
//! 3. On `AllowAndRemember` writes the name back to the DB so the
//!    allowlist survives session restarts.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ravn_persistence::Db;
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
    db: Db,
    allowlist: Arc<Mutex<HashSet<String>>>,
}

impl TuiApprover {
    pub fn new(db: Db, tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            tx,
            db,
            allowlist: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Populate the in-memory cache from the `tool_allowlist` DB table.
    /// Call this once at startup, before the agent loop runs.
    pub async fn preload(&self) {
        match ravn_persistence::allowlist::list_all(&self.db).await {
            Ok(names) => {
                let mut set = self.allowlist.lock().unwrap();
                for n in names {
                    set.insert(n);
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to preload tool allowlist"),
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
                if let Err(e) = ravn_persistence::allowlist::insert(&self.db, tool).await {
                    tracing::warn!(error = %e, tool = %tool, "failed to persist allowlist entry");
                }
                ApprovalDecision::Allow
            }
            Ok(d) => d,
            Err(_) => ApprovalDecision::Deny,
        }
    }
}
