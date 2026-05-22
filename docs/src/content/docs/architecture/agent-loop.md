---
title: Agent loop
description: ReAct, streaming, budgets, cancellation.
sidebar:
  order: 2
---

The agent loop lives in [`ravn-core::agent`]. It's a hand-written
ReAct implementation — no framework, no graph engine — because we
wanted to fully understand the loop before adding abstractions
(`project.md` §6: simplest solution first).

[`ravn-core::agent`]: https://github.com/mkorbi/ravn/blob/main/crates/core/src/agent.rs

## The loop

Pseudocode:

```rust
loop {
    budget.bump_step()?;     // hard-cap on 50 iterations
    if cancel.is_cancelled() { return Cancelled; }

    let mode = router.classify(step, last_result);   // Fast / Deep / Reflect

    let prompt = PromptBuilder::new()
        .tools(registry.as_schemas())
        .system(&config.system_prompt)
        .memory_md(...).world_md(...).soul_md(...).user_md(...)
        .reasoning_effort(mode.reasoning_effort())
        .history(history.clone())
        .build(model, next_input, max_tokens);

    persist_message(session_id, &next_input); // FTS + vec
    history.push(next_input);

    let assistant = stream_one_turn(prompt).await?;
    budget.add_llm_call(model, &assistant.usage)?;
    persist_message(session_id, &assistant);
    history.push(assistant.clone());

    let (text, tool_uses) = split(assistant);
    if tool_uses.is_empty() {
        return Done { final_text, history };
    }

    let tool_results = run_each(tool_uses, approver, cancel).await;
    next_input = Message::user(tool_results);
}
```

Three things deserve special attention.

### Streaming → buffered tool-use

The model's response streams in as a sequence of `StreamChunk`s:

```
TextDelta("Sure, let me ")
TextDelta("check.")
ToolUseStart { id: "toolu_1", name: "datetime" }
ToolUseDelta { partial_json: "{}" }
ToolUseEnd
Done { finish_reason: ToolUse }
```

Text deltas flow to the TUI right away (`LoopEvent::TextDelta`).
Tool-use blocks are buffered into a `ToolBuf` and finalized at
`ToolUseEnd` into a `ContentBlock::ToolUse`. The buffered approach is
what lets the model do "Sure, let me check." → tool call → text in
the next turn without interleaving.

### Provider-id deduplication

`rmcp` and Anthropic streams sometimes emit both a `ToolCallDelta`
(name + args chunks) **and** a final `ToolCall` for the same
provider-assigned tool id. Naïvely that would create two
`ContentBlock::ToolUse` blocks with identical ids, which Anthropic
rejects on the next turn:

```
messages.N.content.M: tool_use ids must be unique
```

The fix lives in two layers. The provider adapter tracks the
`internal_call_id` of any tool started via deltas (`delta_tool:
Option<String>`) — when the final `ToolCall` arrives with the same id,
it emits `ToolUseEnd` only, not a fresh Start/Delta/End. Then
`agent.rs::stream_one_turn` keeps a `HashSet<String>` of provider ids
already pushed; duplicates beyond the adapter layer get logged and
dropped.

### Budget and cancellation

`BudgetTracker` caps:

- **steps** — default 50. Trips on `bump_step` at the head of each
  iteration.
- **input / output tokens** — default 200 k / 50 k.
- **USD spend** — default $1.00, computed via the pricing table.

A trip emits `LoopEvent::BudgetExceeded` and returns
`AgentError::BudgetExceeded`. The cli shows it as a yellow notice.

`CancellationToken` is shared with the LLM stream (`tokio::select!`
on the stream and the token), with every tool's `ToolContext`, and
with the shell subprocess (`kill_on_drop`). Esc → cancel propagates
in well under a second.

## Reasoning modes, subagents & checkpoints

Three Phase-3 capabilities ride on top of the base loop:

- **Reasoning router (D15).** Before each step a `Router` picks a `Mode` —
  `Fast`, `Deep`, or `Reflect` — from cheap heuristics (step depth, whether the
  last tool returned an error). `Deep` maps to Anthropic **extended thinking**
  (`thinking.budget_tokens`) or OpenAI's `reasoning_effort`; `Reflect` drives a
  self-critique + re-plan after a failure. Classification spends no extra LLM
  call — it's deterministic and easy to debug. A classifier-LLM arrives with
  Phase 6 RL.
- **Subagents.** A run can delegate a focused sub-task (e.g. "find all callers
  of `foo`") via `SubagentTool`. The subagent gets a **read-only** tool subset,
  an isolated `RunContext`, and its own `Budget`, and returns only a summary +
  token count — never raw context. Nested subagents are hard-disabled.
- **Checkpoints.** `ravn-orchestration` serializes each `StateGraph` node
  transition (`postcard`) into the `events` table, so a run killed with
  `kill -9` resumes from its last checkpoint instead of restarting.

## Trajectory events

The `events` table gets one row per ReAct boundary:

| `kind` | Payload |
|---|---|
| `react.tool.start` | `{ name, permission }` |
| `react.tool.end` | `{ name, is_error, len }` |
| `react.done` | `{ steps, cost_usd }` |
| `llm.request` / `llm.response` | model + size |

All keyed by a UUID `trace_id` for replay. Phase 6 turns these into
RL training data.
