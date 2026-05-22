//! ReAct-Loop (Phase 1.1).
//!
//! Single-turn agent: takes a user input, runs `thought → action →
//! observation` until the assistant produces a tool-free response or a
//! budget cap trips. Tool calls are dispatched through
//! [`ravn_tools::ToolRegistry`] with Approver gating on Write/Exec.
//!
//! Streaming: each LLM call streams visible text via
//! [`crate::event::LoopEvent::TextDelta`] so the frontend can render
//! tokens as they arrive. Tool-use blocks are buffered and dispatched
//! at the end of the assistant turn.

use std::collections::HashSet;
use std::sync::Arc;

use futures::StreamExt;
use ravn_embeddings::Embedder;
use ravn_llm::{
    ContentBlock, LlmProvider, Message, PromptBuilder, Role, StreamChunk, Usage,
};
use ravn_memory::SemanticMemory;
use ravn_persistence::{events, messages, vector, Db};
use ravn_tools::{ApprovalDecision, Approver, ToolContext, ToolRegistry};
use tokio_util::sync::CancellationToken;

use crate::budget::{Budget, BudgetTracker, BudgetUsage};
use crate::error::AgentError;
use crate::event::{EventSink, LoopEvent};
use crate::reasoning::Mode;
use crate::router::{HeuristicRouter, Router, RouterInput};

#[derive(Clone)]
pub struct AgentConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub budget: Budget,
    /// Initial reasoning mode. **Note:** since Phase 3.1, the per-step
    /// [`crate::router::Router`] picks the mode on every iteration —
    /// this field is kept for backward compatibility with callers that
    /// haven't migrated to `Agent::with_router`. Use a [`FixedRouter`]
    /// to lock the loop to a specific mode.
    ///
    /// [`FixedRouter`]: crate::router::FixedRouter
    pub mode: Mode,
}

impl AgentConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_prompt: "You are ravn, a concise and helpful assistant.".into(),
            max_tokens: 4096,
            budget: Budget::default(),
            mode: Mode::default(),
        }
    }

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    approver: Arc<dyn Approver>,
    db: Db,
    /// Optional — if `Some`, every persisted message is also embedded and
    /// indexed in `messages_vec` (fire-and-forget). Tests pass `None`.
    embedder: Option<Arc<Embedder>>,
    /// Pre-step Mode classifier. Defaults to [`HeuristicRouter`] (D15).
    router: Arc<dyn Router>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        approver: Arc<dyn Approver>,
        db: Db,
    ) -> Self {
        Self {
            provider,
            tools,
            approver,
            db,
            embedder: None,
            router: Arc::new(HeuristicRouter::default()),
        }
    }

    pub fn with_embedder(mut self, embedder: Arc<Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn with_router(mut self, router: Arc<dyn Router>) -> Self {
        self.router = router;
        self
    }
}

#[derive(Debug, Clone)]
pub struct RunContext {
    pub session_id: String,
    pub trace_id: String,
    pub semantic: SemanticMemory,
    pub history: Vec<Message>,
    pub user_turn: Message,
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub steps: usize,
    pub usage: BudgetUsage,
    pub final_text: String,
    pub history: Vec<Message>,
}

impl Agent {
    pub async fn run(
        &self,
        config: &AgentConfig,
        ctx: RunContext,
        sink: Arc<dyn EventSink>,
        cancel: CancellationToken,
    ) -> Result<RunSummary, AgentError> {
        let mut budget = BudgetTracker::new(config.budget);
        let mut history = ctx.history;
        let mut next_input = ctx.user_turn;
        // Per-step router signals.
        let mut last_iteration_had_tool_error = false;
        let mut previous_mode_was_reflect = false;
        let mut reflection_attempts: usize = 0;
        let mut current_mode;

        loop {
            if let Err(reason) = budget.bump_step() {
                emit(
                    &sink,
                    LoopEvent::BudgetExceeded {
                        reason: reason.into(),
                    },
                )
                .await;
                return Err(AgentError::BudgetExceeded(reason.into()));
            }
            if cancel.is_cancelled() {
                emit(&sink, LoopEvent::Error("cancelled".into())).await;
                return Err(AgentError::Cancelled);
            }
            emit(&sink, LoopEvent::StepStart { step: budget.usage.steps }).await;

            // Router picks the mode for this step (Phase 3.1, D15).
            // Default-builds use HeuristicRouter; tests / advanced
            // callers can swap via Agent::with_router.
            current_mode = self.router.classify(RouterInput {
                step: budget.usage.steps,
                last_iteration_had_tool_error,
                previous_mode_was_reflect,
                reflection_attempts,
            });
            if current_mode == Mode::Reflect {
                reflection_attempts += 1;
                // Prepend a Self-Critique-Prefix to the next user turn
                // (= the aggregated tool_results) so the model is
                // primed to analyze the failure before acting again.
                // Does NOT bust the prompt cache: the cached prefix
                // ends before this user message.
                next_input = prepend_reflection_prefix(next_input, reflection_attempts);
            }
            emit(
                &sink,
                LoopEvent::ModeChange {
                    step: budget.usage.steps,
                    mode: current_mode,
                },
            )
            .await;

            // Build cache-stable prompt.
            let mut pb = PromptBuilder::new().system(&config.system_prompt);
            if let Some(s) = &ctx.semantic.soul {
                pb = pb.soul_md(s);
            }
            if let Some(m) = &ctx.semantic.memory {
                pb = pb.memory_md(m);
            }
            if let Some(u) = &ctx.semantic.user {
                pb = pb.user_md(u);
            }
            pb = pb
                .history(history.clone())
                .tools(self.tools.as_schemas())
                .reasoning_effort(current_mode.reasoning_effort());
            let req = pb.build(&config.model, next_input.clone(), config.max_tokens);

            // Persist the next user-role turn (initial user input or
            // tool-results aggregation) into the messages table — both
            // FTS5 and (if configured) vector index pick it up via the
            // fire-and-forget helper.
            self.persist_message(&ctx.session_id, &next_input).await;
            history.push(next_input.clone());

            // Stream the LLM response, accumulating text / tool-use blocks.
            let assistant = match self.stream_one_turn(req, sink.clone(), &cancel).await {
                Ok(a) => a,
                Err(e) => {
                    emit(&sink, LoopEvent::Error(e.to_string())).await;
                    return Err(e);
                }
            };

            if let Some(u) = assistant.usage {
                if let Err(reason) = budget.add_llm_call(&config.model, &u) {
                    emit(
                        &sink,
                        LoopEvent::BudgetExceeded {
                            reason: reason.clone(),
                        },
                    )
                    .await;
                    return Err(AgentError::BudgetExceeded(reason));
                }
                emit(&sink, LoopEvent::Usage(u)).await;
            }

            let assistant_msg = Message {
                role: Role::Assistant,
                content: assistant.blocks.clone(),
            };
            self.persist_message(&ctx.session_id, &assistant_msg).await;
            history.push(assistant_msg);

            // Split out tool-uses from terminal text.
            let mut tool_uses = Vec::new();
            let mut text_chunks = Vec::new();
            for block in &assistant.blocks {
                match block {
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    ContentBlock::Text { text } => {
                        text_chunks.push(text.as_str());
                    }
                    _ => {}
                }
            }

            if tool_uses.is_empty() {
                let final_text = text_chunks.join("");
                emit(&sink, LoopEvent::Done).await;
                let _ = events::append_json(
                    &self.db,
                    Some(&ctx.trace_id),
                    Some(&ctx.session_id),
                    "react.done",
                    &serde_json::json!({
                        "steps": budget.usage.steps,
                        "cost_usd": budget.usage.cost_usd,
                    }),
                )
                .await;
                return Ok(RunSummary {
                    steps: budget.usage.steps,
                    usage: budget.usage,
                    final_text,
                    history,
                });
            }

            // Execute each tool, accumulate results as the next user turn.
            let mut tool_results = Vec::new();
            for (id, name, input) in tool_uses {
                let tool = match self.tools.get(&name) {
                    Some(t) => t,
                    None => {
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: id,
                            content: format!("unknown tool: {name}"),
                            is_error: true,
                            trustworthy: true,
                        });
                        continue;
                    }
                };

                emit(
                    &sink,
                    LoopEvent::ToolStart {
                        id: id.clone(),
                        name: name.clone(),
                        args: input.clone(),
                        permission: tool.permission(),
                    },
                )
                .await;

                let _ = events::append_json(
                    &self.db,
                    Some(&ctx.trace_id),
                    Some(&ctx.session_id),
                    "react.tool.start",
                    &serde_json::json!({
                        "name": name,
                        "permission": tool.permission(),
                    }),
                )
                .await;

                if tool.permission().needs_approval() {
                    let decision = self
                        .approver
                        .approve(&name, &input, tool.permission())
                        .await;
                    if decision == ApprovalDecision::Deny {
                        emit(
                            &sink,
                            LoopEvent::ToolDenied {
                                id: id.clone(),
                                name: name.clone(),
                            },
                        )
                        .await;
                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: id,
                            content: format!("user denied tool call: {name}"),
                            is_error: true,
                            trustworthy: true,
                        });
                        continue;
                    }
                    // `AllowAndRemember` → Phase 1.9 wires the allowlist
                    // persist path here.
                }

                let tool_ctx = ToolContext::new(
                    self.db.clone(),
                    &ctx.session_id,
                    &ctx.trace_id,
                    self.approver.clone(),
                )
                .with_cancel(cancel.clone());

                let result = tool.invoke(input, &tool_ctx).await;
                let output = match result {
                    Ok(o) => o,
                    Err(e) => ravn_tools::ToolOutput::error(e.to_string()),
                };

                emit(
                    &sink,
                    LoopEvent::ToolEnd {
                        id: id.clone(),
                        name: name.clone(),
                        content: output.content.clone(),
                        is_error: output.is_error,
                        trustworthy: output.trustworthy,
                    },
                )
                .await;

                let _ = events::append_json(
                    &self.db,
                    Some(&ctx.trace_id),
                    Some(&ctx.session_id),
                    "react.tool.end",
                    &serde_json::json!({
                        "name": name,
                        "is_error": output.is_error,
                        "len": output.content.len(),
                    }),
                )
                .await;

                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: id,
                    content: output.content,
                    is_error: output.is_error,
                    trustworthy: output.trustworthy,
                });
            }

            // Router signals for the *next* iteration:
            // - any tool returning is_error makes the next step Reflect
            //   (or escalates to Deep if we were already in Reflect).
            // - track whether this iteration was Reflect so the escalation
            //   in HeuristicRouter::classify works.
            last_iteration_had_tool_error = next_input_has_tool_error(&tool_results);
            previous_mode_was_reflect = current_mode == Mode::Reflect;

            next_input = Message {
                role: Role::User,
                content: tool_results,
            };
        }
    }

    /// Persist a `Message` to the `messages` table (full content as JSON
    /// so FTS5 indexes it) and, if an embedder is configured, fire-and-
    /// forget an embed-and-insert into `messages_vec`. Best-effort —
    /// errors are logged but don't abort the agent loop.
    async fn persist_message(&self, session_id: &str, msg: &Message) {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let content_json = match serde_json::to_string(&msg.content) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "serialize message content");
                return;
            }
        };
        let rowid = match messages::append(&self.db, session_id, role, &content_json).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "messages::append");
                return;
            }
        };
        if let Some(embedder) = self.embedder.clone() {
            let db = self.db.clone();
            let text = extract_text(&msg.content);
            if text.is_empty() {
                return;
            }
            tokio::spawn(async move {
                match embedder.embed(vec![text]).await {
                    Ok(mut vecs) => {
                        if let Some(v) = vecs.pop() {
                            if let Err(e) = vector::insert(
                                &db,
                                vector::VecTable::Messages,
                                rowid,
                                &v,
                            )
                            .await
                            {
                                tracing::warn!(error = %e, rowid, "vector insert");
                            }
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "embedder.embed"),
                }
            });
        }
    }

    async fn stream_one_turn(
        &self,
        req: ravn_llm::CompletionRequest,
        sink: Arc<dyn EventSink>,
        cancel: &CancellationToken,
    ) -> Result<AssistantTurn, AgentError> {
        let mut stream = self.provider.stream(req);

        let mut text = String::new();
        let mut thinking = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut current_tool: Option<ToolBuf> = None;
        let mut completed_tools: Vec<ContentBlock> = Vec::new();
        // Defense-in-depth dedup: adapters should not emit two
        // ToolUseStart/End pairs for the same provider tool id, but if
        // they do (e.g. rig emits both ToolCallDelta + final ToolCall
        // for Anthropic), we still drop the duplicate here so the next
        // model turn doesn't fail the "tool_use ids must be unique"
        // server-side check.
        let mut seen_tool_ids: HashSet<String> = HashSet::new();
        let mut usage: Option<Usage> = None;

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                next = stream.next() => match next {
                    None => break,
                    Some(Err(e)) => return Err(e.into()),
                    Some(Ok(chunk)) => match chunk {
                        StreamChunk::TextDelta(t) => {
                            text.push_str(&t);
                            emit(&sink, LoopEvent::TextDelta(t)).await;
                        }
                        StreamChunk::ThinkingDelta(t) => {
                            thinking.push_str(&t);
                            emit(&sink, LoopEvent::ThinkingDelta(t)).await;
                        }
                        StreamChunk::ThinkingSignature(sig) => {
                            // Anthropic requires this back on the next turn
                            // or the API returns 400. Last writer wins —
                            // Anthropic only emits one block per turn in
                            // practice; OpenAI's Text reasoning is
                            // signature-less so this stays None there.
                            thinking_signature = sig;
                        }
                        StreamChunk::ToolUseStart { id, name } => {
                            if let Some(prev) = current_tool.take() {
                                push_unique_tool(
                                    &mut completed_tools,
                                    &mut seen_tool_ids,
                                    finalize_tool(prev),
                                );
                            }
                            if seen_tool_ids.contains(&id) {
                                // We've already finalized this id —
                                // ignore the duplicate Start so the
                                // following Delta/End updates are
                                // discarded too.
                                current_tool = None;
                            } else {
                                current_tool = Some(ToolBuf {
                                    id,
                                    name,
                                    json: String::new(),
                                });
                            }
                        }
                        StreamChunk::ToolUseDelta { partial_json } => {
                            if let Some(t) = current_tool.as_mut() {
                                t.json.push_str(&partial_json);
                            }
                        }
                        StreamChunk::ToolUseEnd => {
                            if let Some(t) = current_tool.take() {
                                push_unique_tool(
                                    &mut completed_tools,
                                    &mut seen_tool_ids,
                                    finalize_tool(t),
                                );
                            }
                        }
                        StreamChunk::Usage(u) => {
                            usage = Some(u);
                        }
                        StreamChunk::Done { .. } => break,
                    }
                }
            }
        }
        if let Some(t) = current_tool.take() {
            push_unique_tool(
                &mut completed_tools,
                &mut seen_tool_ids,
                finalize_tool(t),
            );
        }

        let mut blocks = Vec::new();
        if !thinking.is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking,
                signature: thinking_signature,
            });
        }
        if !text.is_empty() {
            blocks.push(ContentBlock::Text { text });
        }
        blocks.extend(completed_tools);

        Ok(AssistantTurn { blocks, usage })
    }
}

struct ToolBuf {
    id: String,
    name: String,
    json: String,
}

fn finalize_tool(t: ToolBuf) -> ContentBlock {
    let input: serde_json::Value = if t.json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&t.json).unwrap_or_else(|_| serde_json::json!({"_raw": t.json}))
    };
    ContentBlock::ToolUse {
        id: t.id,
        name: t.name,
        input,
    }
}

/// `true` if any tool-result block in the slice signaled an error.
/// Used by the router to switch to [`Mode::Reflect`] on the next step.
fn next_input_has_tool_error(blocks: &[ContentBlock]) -> bool {
    blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolResult { is_error: true, .. }))
}

/// Prepend a Self-Critique-Prefix to a user-role message containing
/// tool results. The prefix tells the model: a previous call errored,
/// reflect on why, propose a different approach. Used in
/// [`Mode::Reflect`] (Phase 3.5).
fn prepend_reflection_prefix(msg: Message, attempt: usize) -> Message {
    let prefix = format!(
        "[reflection attempt {attempt}] The previous tool call returned an error. \
         Before retrying, analyze briefly what went wrong, then propose a different \
         approach — do not repeat the exact same call. If you cannot recover, say so."
    );
    let mut content = Vec::with_capacity(msg.content.len() + 1);
    content.push(ContentBlock::Text { text: prefix });
    content.extend(msg.content);
    Message {
        role: msg.role,
        content,
    }
}

/// Concatenate the searchable text from a list of content blocks for
/// embedding. Tool-use blocks (JSON args) and thinking deltas are
/// excluded — only what a human would see as the message body.
fn extract_text(blocks: &[ContentBlock]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for b in blocks {
        match b {
            ContentBlock::Text { text } => parts.push(text.as_str()),
            ContentBlock::ToolResult { content, .. } => parts.push(content.as_str()),
            ContentBlock::ToolUse { .. } | ContentBlock::Thinking { .. } => {}
        }
    }
    parts.join("\n")
}

fn push_unique_tool(
    completed: &mut Vec<ContentBlock>,
    seen: &mut HashSet<String>,
    block: ContentBlock,
) {
    if let ContentBlock::ToolUse { id, .. } = &block {
        if seen.insert(id.clone()) {
            completed.push(block);
        } else {
            tracing::warn!(tool_id = %id, "dropped duplicate tool_use block from stream");
        }
    }
}

struct AssistantTurn {
    blocks: Vec<ContentBlock>,
    usage: Option<Usage>,
}

async fn emit(sink: &Arc<dyn EventSink>, event: LoopEvent) {
    sink.emit(event).await;
}
