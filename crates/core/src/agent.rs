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

use std::sync::Arc;

use futures::StreamExt;
use ravn_llm::{
    ContentBlock, LlmProvider, Message, PromptBuilder, Role, StreamChunk, Usage,
};
use ravn_memory::SemanticMemory;
use ravn_persistence::{events, Db};
use ravn_tools::{ApprovalDecision, Approver, ToolContext, ToolRegistry};
use tokio_util::sync::CancellationToken;

use crate::budget::{Budget, BudgetTracker, BudgetUsage};
use crate::error::AgentError;
use crate::event::{EventSink, LoopEvent};

#[derive(Clone)]
pub struct AgentConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub budget: Budget,
}

impl AgentConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system_prompt: "You are ravn, a concise and helpful assistant.".into(),
            max_tokens: 4096,
            budget: Budget::default(),
        }
    }
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    approver: Arc<dyn Approver>,
    db: Db,
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
        }
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
            pb = pb.history(history.clone()).tools(self.tools.as_schemas());
            let req = pb.build(&config.model, next_input.clone(), config.max_tokens);

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

            next_input = Message {
                role: Role::User,
                content: tool_results,
            };
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
        let mut current_tool: Option<ToolBuf> = None;
        let mut completed_tools: Vec<ContentBlock> = Vec::new();
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
                        StreamChunk::ToolUseStart { id, name } => {
                            if let Some(prev) = current_tool.take() {
                                completed_tools.push(finalize_tool(prev));
                            }
                            current_tool = Some(ToolBuf {
                                id,
                                name,
                                json: String::new(),
                            });
                        }
                        StreamChunk::ToolUseDelta { partial_json } => {
                            if let Some(t) = current_tool.as_mut() {
                                t.json.push_str(&partial_json);
                            }
                        }
                        StreamChunk::ToolUseEnd => {
                            if let Some(t) = current_tool.take() {
                                completed_tools.push(finalize_tool(t));
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
            completed_tools.push(finalize_tool(t));
        }

        let mut blocks = Vec::new();
        if !thinking.is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking,
                signature: None,
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

struct AssistantTurn {
    blocks: Vec<ContentBlock>,
    usage: Option<Usage>,
}

async fn emit(sink: &Arc<dyn EventSink>, event: LoopEvent) {
    sink.emit(event).await;
}
