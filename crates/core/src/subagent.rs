//! Subagent delegation (Phase 3.8 / 3.9 / 3.10, D17).
//!
//! `SubagentTool` is a [`Tool`] that, when invoked, spawns a fresh
//! `Agent::run` with:
//!
//! - **A reduced tool set** — strictly Read-permission, with
//!   `subagent_delegate` itself excluded (D17 hard-prevent nested
//!   delegation; project.md §1.6 "no nested subagents").
//! - **An isolated `RunContext`** — its own `session_id`, derived
//!   `trace_id`, empty conversation history.
//! - **Its own `Budget`** — caller-supplied caps, defaulted small.
//!
//! Returns only the sub-agent's final text answer ([`SubagentResult`])
//! plus a one-line cost stamp — never the raw conversation. The
//! parent agent treats the result like any other tool output.

use std::sync::Arc;

use async_trait::async_trait;
use ravn_embeddings::Embedder;
use ravn_llm::{LlmProvider, Message};
use ravn_memory::SemanticMemory;
use ravn_persistence::Db;
use ravn_tools::{Approver, Permission, Tool, ToolContext, ToolError, ToolOutput, ToolRegistry};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::agent::{Agent, AgentConfig, RunContext};
use crate::budget::{Budget, BudgetUsage};
use crate::event::NullSink;

/// Public name of the delegate tool. Other crates can refer to it by
/// this constant to exclude it from a read-only subset (D17).
pub const SUBAGENT_TOOL_NAME: &str = "subagent_delegate";

#[derive(Debug, Clone)]
pub struct SubagentResult {
    pub summary: String,
    pub usage: BudgetUsage,
    pub steps: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Concrete task the sub-agent should accomplish. Be specific —
    /// the sub-agent has no conversation context to fall back on.
    goal: String,
    /// Hard cap on the sub-agent's loop iterations. Default 20.
    #[serde(default)]
    max_steps: Option<usize>,
    /// Hard cap on the sub-agent's total spend in USD. Default 0.25.
    #[serde(default)]
    max_cost_usd: Option<f64>,
}

/// Tool that delegates a sub-task to a separate, read-only Agent.
pub struct SubagentTool {
    provider: Arc<dyn LlmProvider>,
    /// Tool surface for the sub-agent. **Must** be a read-only subset
    /// and **must not** contain another `SubagentTool` — D17.
    sub_tools: Arc<ToolRegistry>,
    approver: Arc<dyn Approver>,
    db: Db,
    embedder: Option<Arc<Embedder>>,
    model: String,
    system_prompt: String,
}

impl SubagentTool {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        sub_tools: Arc<ToolRegistry>,
        approver: Arc<dyn Approver>,
        db: Db,
        model: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            sub_tools,
            approver,
            db,
            embedder: None,
            model: model.into(),
            system_prompt: default_system_prompt(),
        }
    }

    pub fn with_embedder(mut self, embedder: Arc<Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }
}

fn default_system_prompt() -> String {
    "You are a sub-agent. Your job is to answer one focused, read-only \
     question and return a concise summary. You have access only to \
     read tools; you cannot write files, run shell commands, or delegate \
     further. Keep your response under 500 tokens unless the task \
     specifically requires more detail."
        .to_string()
}

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &'static str {
        SUBAGENT_TOOL_NAME
    }

    fn description(&self) -> &'static str {
        "Delegate a focused, read-only sub-task to a separate agent with its own context and budget. \
         Use this for exploratory work that would otherwise pollute the main conversation \
         (e.g. \"find all callers of foo\", \"summarize this file's intent\", \"check whether X is mentioned anywhere in past sessions\"). \
         The sub-agent has only read tools — no file writes, no shell, no nested delegation. \
         Returns a concise summary plus a cost stamp."
    }

    fn permission(&self) -> Permission {
        // Read: the sub-agent itself runs only Read tools, so the
        // delegate call as a whole is side-effect-free from the
        // parent's perspective.
        Permission::Read
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(Args)).unwrap_or_default()
    }

    async fn invoke(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;

        let mut budget = Budget {
            max_steps: args.max_steps.unwrap_or(20),
            max_cost_usd: args.max_cost_usd.unwrap_or(0.25),
            ..Budget::default()
        };
        // Cap tokens too to avoid a runaway long-form sub-conversation.
        budget.max_input_tokens = 100_000;
        budget.max_output_tokens = 25_000;

        let sub_session_id = format!("sub-{}", uuid::Uuid::new_v4());
        let sub_trace_id = format!("{}/{}", ctx.trace_id, &sub_session_id[..12]);

        // The sub-session is recorded in the same DB so its messages
        // and events show up in `session_search` later.
        ravn_persistence::sessions::create(
            &self.db,
            &sub_session_id,
            "subagent",
            Some(&self.model),
        )
        .await
        .map_err(|e| ToolError::Internal(format!("create sub-session: {e}")))?;

        let mut sub_agent = Agent::new(
            self.provider.clone(),
            self.sub_tools.clone(),
            self.approver.clone(),
            self.db.clone(),
        );
        if let Some(emb) = &self.embedder {
            sub_agent = sub_agent.with_embedder(emb.clone());
        }

        let config = AgentConfig {
            model: self.model.clone(),
            system_prompt: self.system_prompt.clone(),
            max_tokens: 4096,
            budget,
            mode: crate::reasoning::Mode::Fast,
        };

        let run_ctx = RunContext {
            session_id: sub_session_id.clone(),
            trace_id: sub_trace_id,
            semantic: SemanticMemory::default(),
            history: Vec::new(),
            user_turn: Message::user(args.goal.clone()),
        };

        let cancel = ctx.cancel.clone();
        let summary = sub_agent
            .run(&config, run_ctx, Arc::new(NullSink), cancel)
            .await
            .map_err(|e| ToolError::Internal(format!("subagent: {e}")))?;

        let _ = ravn_persistence::sessions::close(&self.db, &sub_session_id).await;

        let body = format!(
            "[subagent: {} steps, {} input + {} output tokens, ${:.4}]\n\n{}",
            summary.steps,
            summary.usage.input_tokens,
            summary.usage.output_tokens,
            summary.usage.cost_usd,
            summary.final_text,
        );
        Ok(ToolOutput::ok(body))
    }
}

// Integration-test of an actual delegation lives in
// `crates/core/src/tests.rs::subagent_delegates_to_sub_loop`.
