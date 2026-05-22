//! Integration tests for the ReAct loop.
//!
//! Uses a scriptable [`MockProvider`] so the loop can be exercised end-to-end
//! without hitting a real LLM. Each test calls `Agent::run` and asserts on
//! the emitted events and the `RunSummary`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use ravn_llm::provider::Error as LlmError;
use ravn_llm::{
    message::Role, response::FinishReason, CompletionRequest, CompletionResponse, ContentBlock,
    LlmProvider, Message, StreamChunk, Usage,
};
use ravn_memory::SemanticMemory;
use ravn_persistence::Db;
use ravn_tools::{
    AllowAll, Approver, DenyAll, Permission, Tool, ToolContext, ToolError, ToolOutput,
    ToolRegistry,
};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{Agent, AgentConfig, Budget, ChannelSink, LoopEvent, RunContext};

// --- mock provider -----------------------------------------------------

/// Scripted provider: each call to `stream()` returns the next pre-built
/// sequence of [`StreamChunk`]s.
#[derive(Clone)]
struct MockProvider {
    scripts: Arc<Mutex<Vec<Vec<StreamChunk>>>>,
}

impl MockProvider {
    fn new(scripts: Vec<Vec<StreamChunk>>) -> Self {
        Self {
            scripts: Arc::new(Mutex::new(scripts)),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn supports_caching(&self) -> bool {
        false
    }
    fn supports_reasoning(&self) -> bool {
        false
    }

    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::InvalidRequest("mock complete unsupported".into()))
    }

    fn stream(
        &self,
        _req: CompletionRequest,
    ) -> BoxStream<'static, Result<StreamChunk, LlmError>> {
        let mut s = self.scripts.lock().unwrap();
        if s.is_empty() {
            return Box::pin(stream::once(async {
                Err(LlmError::InvalidRequest("no more scripted turns".into()))
            }));
        }
        let chunks = s.remove(0);
        let owned: Vec<Result<StreamChunk, LlmError>> = chunks.into_iter().map(Ok).collect();
        Box::pin(stream::iter(owned))
    }
}

// --- mock tools --------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct AddArgs {
    a: i64,
    b: i64,
}

struct AddTool;

#[async_trait]
impl Tool for AddTool {
    fn name(&self) -> &'static str {
        "add"
    }
    fn description(&self) -> &'static str {
        "Add two integers"
    }
    fn permission(&self) -> Permission {
        Permission::Read
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(AddArgs)).unwrap_or_default()
    }
    async fn invoke(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: AddArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        Ok(ToolOutput::ok((args.a + args.b).to_string()))
    }
}

struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Pretend to write a file"
    }
    fn permission(&self) -> Permission {
        Permission::Write
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn invoke(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::ok("wrote 42 bytes"))
    }
}

// --- harness ----------------------------------------------------------

async fn harness(
    scripts: Vec<Vec<StreamChunk>>,
    tools: ToolRegistry,
    approver: Arc<dyn Approver>,
) -> (Agent, RunContext, AgentConfig, CancellationToken) {
    let db = Db::open_in_memory().await.unwrap();
    ravn_persistence::sessions::create(&db, "sess-1", "test", Some("mock"))
        .await
        .unwrap();

    let provider = Arc::new(MockProvider::new(scripts));
    let agent = Agent::new(provider, Arc::new(tools), approver, db);

    let config = AgentConfig {
        budget: Budget {
            max_steps: 5,
            ..Budget::default()
        },
        ..AgentConfig::new("mock-model")
    };

    let ctx = RunContext {
        session_id: "sess-1".into(),
        trace_id: "trace-1".into(),
        semantic: SemanticMemory::default(),
        history: Vec::new(),
        user_turn: Message::user("hi"),
    };

    (agent, ctx, config, CancellationToken::new())
}

// --- tests ------------------------------------------------------------

#[tokio::test]
async fn text_only_response_terminates() {
    let scripts = vec![vec![
        StreamChunk::TextDelta("hello".into()),
        StreamChunk::TextDelta(" world".into()),
        StreamChunk::Usage(Usage {
            input_tokens: 50,
            output_tokens: 5,
            ..Default::default()
        }),
        StreamChunk::Done {
            finish_reason: FinishReason::Stop,
        },
    ]];
    let (agent, ctx, cfg, cancel) =
        harness(scripts, ToolRegistry::new(), Arc::new(AllowAll)).await;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    assert_eq!(summary.final_text, "hello world");
    assert_eq!(summary.steps, 1);
    assert_eq!(summary.usage.input_tokens, 50);

    let events = collect_recv(&mut rx).await;
    assert!(events.iter().any(|e| matches!(e, LoopEvent::TextDelta(_))));
    assert!(events.iter().any(|e| matches!(e, LoopEvent::Done)));
}

#[tokio::test]
async fn read_tool_call_then_text() {
    let mut tools = ToolRegistry::new();
    tools.register(AddTool);

    // First turn: assistant emits a tool_use.
    // Second turn: assistant returns final text after seeing tool result.
    let scripts = vec![
        vec![
            StreamChunk::ToolUseStart {
                id: "toolu_1".into(),
                name: "add".into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: r#"{"a":2,"b":3}"#.into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::Done {
                finish_reason: FinishReason::ToolUse,
            },
        ],
        vec![
            StreamChunk::TextDelta("the sum is 5".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
    ];

    let (agent, ctx, cfg, cancel) = harness(scripts, tools, Arc::new(AllowAll)).await;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    assert_eq!(summary.steps, 2);
    assert_eq!(summary.final_text, "the sum is 5");

    let events = collect_recv(&mut rx).await;
    let tool_starts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            LoopEvent::ToolStart { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tool_starts, vec!["add"]);
    let tool_ends: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            LoopEvent::ToolEnd { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tool_ends, vec!["5"]);
}

#[tokio::test]
async fn write_tool_denied_by_approver() {
    let mut tools = ToolRegistry::new();
    tools.register(WriteFileTool);

    let scripts = vec![
        vec![
            StreamChunk::ToolUseStart {
                id: "toolu_1".into(),
                name: "write_file".into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: r#"{}"#.into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::Done {
                finish_reason: FinishReason::ToolUse,
            },
        ],
        vec![
            StreamChunk::TextDelta("ok, skipped.".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
    ];

    let (agent, ctx, cfg, cancel) = harness(scripts, tools, Arc::new(DenyAll)).await;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();
    assert_eq!(summary.final_text, "ok, skipped.");

    let events = collect_recv(&mut rx).await;
    assert!(events
        .iter()
        .any(|e| matches!(e, LoopEvent::ToolDenied { name, .. } if name == "write_file")));
    assert!(!events
        .iter()
        .any(|e| matches!(e, LoopEvent::ToolEnd { name, .. } if name == "write_file")));

    // History second turn must contain the ToolResult marked as error.
    let tool_result_block = summary.history.iter().rev().find_map(|m| {
        if m.role == Role::User {
            m.content.iter().find_map(|b| match b {
                ContentBlock::ToolResult {
                    is_error, content, ..
                } => Some((*is_error, content.clone())),
                _ => None,
            })
        } else {
            None
        }
    });
    let (is_error, content) = tool_result_block.expect("tool result in history");
    assert!(is_error);
    assert!(content.contains("denied"));
}

#[tokio::test]
async fn budget_max_steps_trips() {
    // Provider always asks to call `add` — never terminates with text.
    let one_call = vec![
        StreamChunk::ToolUseStart {
            id: "x".into(),
            name: "add".into(),
        },
        StreamChunk::ToolUseDelta {
            partial_json: r#"{"a":1,"b":1}"#.into(),
        },
        StreamChunk::ToolUseEnd,
        StreamChunk::Done {
            finish_reason: FinishReason::ToolUse,
        },
    ];
    let scripts: Vec<Vec<StreamChunk>> = (0..10).map(|_| one_call.clone()).collect();

    let mut tools = ToolRegistry::new();
    tools.register(AddTool);

    let (agent, ctx, mut cfg, cancel) = harness(scripts, tools, Arc::new(AllowAll)).await;
    cfg.budget.max_steps = 3;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let err = agent.run(&cfg, ctx, sink, cancel).await.unwrap_err();
    assert!(matches!(err, crate::AgentError::BudgetExceeded(_)));

    let events = collect_recv(&mut rx).await;
    assert!(events
        .iter()
        .any(|e| matches!(e, LoopEvent::BudgetExceeded { .. })));
}

#[tokio::test]
async fn subagent_delegates_to_sub_loop() {
    use crate::subagent::{SubagentTool, SUBAGENT_TOOL_NAME};
    use ravn_persistence::Db;

    // Parent provider script: assistant calls subagent_delegate once,
    // then summarizes after seeing the sub-agent's result. Two turns.
    // Sub provider script: assistant just answers the goal directly,
    // no tool use. One turn.
    //
    // We give the parent's MockProvider 3 scripts so the sub-agent's
    // single call also pulls from the same script queue (the
    // MockProvider is shared between parent and sub).
    let scripts = vec![
        // turn 1 (parent): call subagent_delegate
        vec![
            StreamChunk::ToolUseStart {
                id: "toolu_sub".into(),
                name: SUBAGENT_TOOL_NAME.into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: r#"{"goal":"count to three"}"#.into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::Done {
                finish_reason: FinishReason::ToolUse,
            },
        ],
        // turn 1 (sub-agent): plain text answer
        vec![
            StreamChunk::TextDelta("one two three.".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
        // turn 2 (parent): final answer
        vec![
            StreamChunk::TextDelta("sub said: counted.".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
    ];

    let db = Db::open_in_memory().await.unwrap();
    ravn_persistence::sessions::create(&db, "sess-1", "test", Some("mock"))
        .await
        .unwrap();
    let provider: Arc<dyn ravn_llm::LlmProvider> = Arc::new(MockProvider::new(scripts));

    // The sub-agent's tool surface is empty here (no Read tools needed
    // for "count to three"). It explicitly does NOT include
    // SubagentTool itself — that's the D17 nested-prevention.
    let sub_tools = Arc::new(ToolRegistry::new());

    // Parent registry: only SubagentTool. We give the sub-agent the
    // same shared provider.
    let mut parent_tools = ToolRegistry::new();
    parent_tools.register(
        SubagentTool::new(
            provider.clone(),
            sub_tools,
            Arc::new(AllowAll),
            db.clone(),
            "mock-model",
        ),
    );

    let agent = Agent::new(
        provider,
        Arc::new(parent_tools),
        Arc::new(AllowAll),
        db.clone(),
    );

    let config = AgentConfig {
        budget: Budget {
            max_steps: 5,
            ..Budget::default()
        },
        ..AgentConfig::new("mock-model")
    };
    let ctx = RunContext {
        session_id: "sess-1".into(),
        trace_id: "trace-1".into(),
        semantic: SemanticMemory::default(),
        history: Vec::new(),
        user_turn: Message::user("delegate the counting"),
    };
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&config, ctx, sink, cancel).await.unwrap();

    // Parent agent saw the sub-agent's result and finalized.
    assert_eq!(summary.final_text, "sub said: counted.");

    // Tool events show subagent_delegate ran exactly once.
    let events = collect_recv(&mut rx).await;
    let sub_starts: usize = events
        .iter()
        .filter(|e| matches!(e, LoopEvent::ToolStart { name, .. } if name == SUBAGENT_TOOL_NAME))
        .count();
    assert_eq!(sub_starts, 1);

    // Sub-session row was persisted (channel = "subagent").
    let recent =
        ravn_persistence::sessions::recent(&db, 10).await.unwrap();
    let sub_sessions: Vec<_> = recent
        .iter()
        .filter(|s| s.channel == "subagent")
        .collect();
    assert_eq!(sub_sessions.len(), 1);
}

#[tokio::test]
async fn router_emits_mode_change_per_step() {
    // Plain happy-path: step 1 should classify as Fast (D15 heuristic).
    let scripts = vec![vec![
        StreamChunk::TextDelta("hi".into()),
        StreamChunk::Done {
            finish_reason: FinishReason::Stop,
        },
    ]];
    let (agent, ctx, cfg, cancel) =
        harness(scripts, ToolRegistry::new(), Arc::new(AllowAll)).await;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    agent.run(&cfg, ctx, sink, cancel).await.unwrap();
    let events = collect_recv(&mut rx).await;
    let mode_changes: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            LoopEvent::ModeChange { step, mode } => Some((*step, *mode)),
            _ => None,
        })
        .collect();
    assert_eq!(mode_changes, vec![(1, crate::ReasoningMode::Fast)]);
}

#[tokio::test]
async fn router_picks_reflect_after_tool_error() {
    use crate::reasoning::Mode;

    // Tool that always errors.
    struct ErroringTool;
    #[async_trait::async_trait]
    impl ravn_tools::Tool for ErroringTool {
        fn name(&self) -> &'static str {
            "explode"
        }
        fn description(&self) -> &'static str {
            "always errors"
        }
        fn permission(&self) -> ravn_tools::Permission {
            ravn_tools::Permission::Read
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }
        async fn invoke(
            &self,
            _args: serde_json::Value,
            _ctx: &ravn_tools::ToolContext,
        ) -> Result<ravn_tools::ToolOutput, ravn_tools::ToolError> {
            Ok(ravn_tools::ToolOutput::error("boom"))
        }
    }

    let mut tools = ToolRegistry::new();
    tools.register(ErroringTool);

    // Turn 1: model calls explode (returns error).
    // Turn 2: model gives up with text — but router should have switched
    // it into Reflect mode pre-step.
    let scripts = vec![
        vec![
            StreamChunk::ToolUseStart {
                id: "toolu_1".into(),
                name: "explode".into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: "{}".into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::Done {
                finish_reason: FinishReason::ToolUse,
            },
        ],
        vec![
            StreamChunk::TextDelta("giving up.".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
    ];

    let (agent, ctx, cfg, cancel) = harness(scripts, tools, Arc::new(AllowAll)).await;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));
    agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    let events = collect_recv(&mut rx).await;
    let mode_changes: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            LoopEvent::ModeChange { step, mode } => Some((*step, *mode)),
            _ => None,
        })
        .collect();
    assert_eq!(mode_changes.len(), 2);
    assert_eq!(mode_changes[0], (1, Mode::Fast));
    assert_eq!(mode_changes[1], (2, Mode::Reflect));
}

#[tokio::test]
async fn reflect_mode_prepends_self_critique_prefix() {
    // Tool that always errors → triggers Reflect mode on next step.
    struct ErroringTool;
    #[async_trait::async_trait]
    impl ravn_tools::Tool for ErroringTool {
        fn name(&self) -> &'static str {
            "explode"
        }
        fn description(&self) -> &'static str {
            "always errors"
        }
        fn permission(&self) -> ravn_tools::Permission {
            ravn_tools::Permission::Read
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }
        async fn invoke(
            &self,
            _args: serde_json::Value,
            _ctx: &ravn_tools::ToolContext,
        ) -> Result<ravn_tools::ToolOutput, ravn_tools::ToolError> {
            Ok(ravn_tools::ToolOutput::error("boom"))
        }
    }

    let mut tools = ToolRegistry::new();
    tools.register(ErroringTool);

    // Turn 1: model calls explode → tool returns is_error=true.
    // Turn 2: router picks Reflect → loop prepends critique prefix → model gives up.
    let scripts = vec![
        vec![
            StreamChunk::ToolUseStart {
                id: "toolu_1".into(),
                name: "explode".into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: "{}".into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::Done {
                finish_reason: FinishReason::ToolUse,
            },
        ],
        vec![
            StreamChunk::TextDelta("giving up.".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
    ];

    let (agent, ctx, cfg, cancel) = harness(scripts, tools, Arc::new(AllowAll)).await;
    let (tx, _rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));
    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    // The reflect-mode user turn (= 2nd user message in history) must
    // start with the self-critique prefix.
    let reflect_input = summary
        .history
        .iter()
        .filter(|m| m.role == Role::User)
        .nth(1)
        .expect("second user turn (= tool results) in history");
    let first_text = reflect_input
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .expect("text block at start of reflect-mode user turn");
    assert!(
        first_text.contains("reflection attempt"),
        "expected self-critique prefix, got: {first_text}"
    );
}

#[tokio::test]
async fn fixed_router_overrides_classification() {
    use crate::reasoning::Mode;
    use crate::router::FixedRouter;

    let scripts = vec![vec![
        StreamChunk::TextDelta("hi".into()),
        StreamChunk::Done {
            finish_reason: FinishReason::Stop,
        },
    ]];
    let db = Db::open_in_memory().await.unwrap();
    ravn_persistence::sessions::create(&db, "sess-1", "test", Some("mock"))
        .await
        .unwrap();
    let provider = Arc::new(MockProvider::new(scripts));
    let agent = Agent::new(provider, Arc::new(ToolRegistry::new()), Arc::new(AllowAll), db)
        .with_router(Arc::new(FixedRouter(Mode::Deep)));

    let cfg = AgentConfig {
        budget: Budget {
            max_steps: 3,
            ..Budget::default()
        },
        ..AgentConfig::new("mock-model")
    };
    let ctx = RunContext {
        session_id: "sess-1".into(),
        trace_id: "trace-1".into(),
        semantic: SemanticMemory::default(),
        history: Vec::new(),
        user_turn: Message::user("hi"),
    };
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));
    agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    let events = collect_recv(&mut rx).await;
    assert_eq!(
        events.iter().find_map(|e| match e {
            LoopEvent::ModeChange { mode, .. } => Some(*mode),
            _ => None,
        }),
        Some(Mode::Deep)
    );
}

#[tokio::test]
async fn thinking_signature_survives_to_history() {
    // Phase 3.4: Anthropic Extended Thinking requires the signature
    // be sent back on the next turn or the API returns 400. The
    // stream layer extracts it from the complete Reasoning block
    // and the agent loop must attach it to ContentBlock::Thinking.
    let scripts = vec![vec![
        StreamChunk::ThinkingDelta("let me think…".into()),
        StreamChunk::ThinkingDelta(" about this".into()),
        StreamChunk::ThinkingSignature(Some("sig-abc-123".into())),
        StreamChunk::TextDelta("done.".into()),
        StreamChunk::Done {
            finish_reason: FinishReason::Stop,
        },
    ]];
    let (agent, ctx, cfg, cancel) =
        harness(scripts, ToolRegistry::new(), Arc::new(AllowAll)).await;
    let (tx, _rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    let assistant = summary
        .history
        .iter()
        .find(|m| m.role == Role::Assistant)
        .expect("assistant message in history");
    let thinking_block = assistant
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Thinking { thinking, signature } => {
                Some((thinking.clone(), signature.clone()))
            }
            _ => None,
        })
        .expect("Thinking block preserved in history");
    assert_eq!(thinking_block.0, "let me think… about this");
    assert_eq!(thinking_block.1.as_deref(), Some("sig-abc-123"));
}

#[tokio::test]
async fn thinking_without_signature_is_ok() {
    // OpenAI o-series Text reasoning is signature-less; we still want
    // to keep the thinking text in history so the next turn can see it,
    // just without the signature field.
    let scripts = vec![vec![
        StreamChunk::ThinkingDelta("hmm".into()),
        StreamChunk::ThinkingSignature(None),
        StreamChunk::TextDelta("ok".into()),
        StreamChunk::Done {
            finish_reason: FinishReason::Stop,
        },
    ]];
    let (agent, ctx, cfg, cancel) =
        harness(scripts, ToolRegistry::new(), Arc::new(AllowAll)).await;
    let (tx, _rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();
    let assistant = summary
        .history
        .iter()
        .find(|m| m.role == Role::Assistant)
        .unwrap();
    let block = assistant
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Thinking { thinking, signature } => {
                Some((thinking.clone(), signature.clone()))
            }
            _ => None,
        })
        .expect("Thinking block preserved even without signature");
    assert_eq!(block.0, "hmm");
    assert!(block.1.is_none());
}

#[tokio::test]
async fn duplicate_tool_use_id_in_stream_is_deduped() {
    // Reproduces the Anthropic-streaming bug: rig emits ToolCallDelta
    // (Start + Delta) AND a final ToolCall for the same provider id,
    // which historically produced two ContentBlock::ToolUse blocks
    // with identical ids and made the next-turn API call fail with
    // "tool_use ids must be unique" (and also ran the tool twice).
    let mut tools = ToolRegistry::new();
    tools.register(AddTool);

    let scripts = vec![
        // First turn: stream emits Start+Delta+End for "add" (the
        // delta path), then Start+Delta+End again for the SAME id
        // (the final-ToolCall path). Both with id "toolu_dup".
        vec![
            StreamChunk::ToolUseStart {
                id: "toolu_dup".into(),
                name: "add".into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: r#"{"a":2,"b":3}"#.into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::ToolUseStart {
                id: "toolu_dup".into(),
                name: "add".into(),
            },
            StreamChunk::ToolUseDelta {
                partial_json: r#"{"a":2,"b":3}"#.into(),
            },
            StreamChunk::ToolUseEnd,
            StreamChunk::Done {
                finish_reason: FinishReason::ToolUse,
            },
        ],
        vec![
            StreamChunk::TextDelta("5".into()),
            StreamChunk::Done {
                finish_reason: FinishReason::Stop,
            },
        ],
    ];

    let (agent, ctx, cfg, cancel) = harness(scripts, tools, Arc::new(AllowAll)).await;
    let (tx, mut rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let summary = agent.run(&cfg, ctx, sink, cancel).await.unwrap();

    // The tool must have run exactly once even though the stream
    // contained a duplicate Start/End pair for the same id.
    let events = collect_recv(&mut rx).await;
    let tool_ends: usize = events
        .iter()
        .filter(|e| matches!(e, LoopEvent::ToolEnd { name, .. } if name == "add"))
        .count();
    assert_eq!(tool_ends, 1, "tool ran more than once on duplicate stream");

    // And the assistant message in history must have a single
    // ContentBlock::ToolUse with that id — the Anthropic API would
    // reject anything else on the next turn.
    let assistant_msg = summary
        .history
        .iter()
        .find(|m| m.role == Role::Assistant)
        .expect("assistant message in history");
    let tool_use_count = assistant_msg
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == "toolu_dup"))
        .count();
    assert_eq!(
        tool_use_count, 1,
        "history contains duplicate tool_use blocks with same id"
    );
}

#[tokio::test]
async fn cancellation_terminates_loop() {
    let scripts = vec![vec![
        StreamChunk::TextDelta("hi".into()),
        StreamChunk::Done {
            finish_reason: FinishReason::Stop,
        },
    ]];
    let (agent, ctx, cfg, cancel) =
        harness(scripts, ToolRegistry::new(), Arc::new(AllowAll)).await;
    cancel.cancel();
    let (tx, _rx) = mpsc::channel(64);
    let sink = Arc::new(ChannelSink::new(tx));

    let err = agent.run(&cfg, ctx, sink, cancel).await.unwrap_err();
    assert!(matches!(err, crate::AgentError::Cancelled));
}

async fn collect_recv(rx: &mut mpsc::Receiver<LoopEvent>) -> Vec<LoopEvent> {
    let mut out = Vec::new();
    rx.close();
    while let Some(e) = rx.recv().await {
        out.push(e);
    }
    out
}
