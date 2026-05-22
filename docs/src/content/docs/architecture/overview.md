---
title: Overview
description: How the workspace fits together.
sidebar:
  order: 1
---

ravn is a Cargo workspace of focused crates. Each crate has one job;
the cli wires them up.

## Crate map

```
              ┌────────────┐
              │  ravn-cli  │  ratatui TUI, slash-commands, approver
              └─────┬──────┘
                    │
       ┌────────────┼─────────────┬──────────────┐
       ▼            ▼             ▼              ▼
┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐
│ ravn-core│  │ravn-tools│  │ ravn-mcp │  │ravn-skills │
└─┬────────┘  └─┬───┬────┘  └─────┬────┘  └─────┬──────┘
  │             │   │             │             │
  │       ┌─────┘   └────────┐    │             │
  ▼       ▼                  ▼    ▼             ▼
┌──────┐ ┌────────────┐    ┌──────────────────────────┐
│ ravn │ │   ravn-    │    │     ravn-persistence     │
│-llm  │ │ embeddings │    │ sqlx + sqlite-vec + FTS5 │
└──┬───┘ └─────┬──────┘    └──────────────────────────┘
   │           │
   ▼           ▼
 rig-core   fastembed
```

| Crate | Responsibility |
|---|---|
| `ravn-llm` | `LlmProvider` trait + OpenAI / Anthropic adapters via [rig-core](https://github.com/0xPlaygrounds/rig). Streaming, cache control, pricing, prompt assembly. |
| `ravn-core` | The ReAct loop. Drives an `Agent` from a `RunContext` to a `RunSummary`, emitting `LoopEvent`s through an `EventSink`. Budget + cancellation. |
| `ravn-tools` | The `Tool` trait + 10 native tools + the `Approver` abstraction + the registry. |
| `ravn-memory` | Semantic memory (soul / memory / user Markdown files) and token-budget enforcement. Working / episodic / procedural layers are stubs for later phases. |
| `ravn-embeddings` | `Embedder` wrapping fastembed's EmbeddingGemma-300M. 768-dim ONNX. Lazy-loaded, single-process. |
| `ravn-mcp` | MCP client via [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 1.x. Each MCP tool wraps as a `ravn_tools::Tool` impl. |
| `ravn-skills` | YAML-frontmatter parser + filesystem → DB sync with SHA-256 change detection. |
| `ravn-persistence` | sqlx for normal CRUD; rusqlite + sqlite-vec for vector ops. WAL mode, append-only events. Also the typed `world_state`. |
| `ravn-orchestration` | Typed `StateGraph` + per-node checkpoints (`postcard` in the `events` table) for crash-resume. |
| `ravn-heartbeat` | Cron scheduler (`tokio-cron-scheduler`) firing unattended runs from `heartbeats.toml`, gated by a per-job allowlist. |
| `ravn-eval` | Hand-crafted eval set + LLM-as-judge runner. |
| `ravn-cli` | The TUI. Approver, splash, slash-commands, input buffer, heartbeat wiring. The only crate that depends on every other crate. |

## Data flow on a user turn

1. User hits Enter in the TUI input pane.
2. `commands::SlashCommand::parse` checks for a `/foo`. If yes, handle
   locally and stop.
3. Otherwise, build a `RunContext` (semantic memory, history, the user
   turn) and spawn `Agent::run` on a tokio task.
4. A per-step router picks a reasoning Mode (Fast / Deep / Reflect),
   then the agent assembles a cache-stable prompt via `PromptBuilder`
   (tools → system → skills → memory → world state → soul → user →
   history → user turn) and sends it to the provider.
5. The streamed response goes through `stream_one_turn`: text deltas
   flow back to the TUI immediately, tool-use blocks buffer.
6. After the stream ends, each tool-use goes through the approver,
   then `invoke()`. Results form the next user turn; the loop iterates.
7. Every step writes to the `events` table for replay and trajectory
   training (Phase 6).
8. When the model emits a text-only turn, `RunSummary` returns to the
   cli, which renders the final message and resets streaming state.

## Why these decisions?

Each architectural choice is captured as a numbered Decision in
[`PLAN.md`](https://github.com/mkorbi/ravn/blob/main/PLAN.md). The big
ones to know:

- **D1**: `rig-core` for provider backends but we own the ReAct loop.
  Lets us upgrade rig and re-shape the loop independently.
- **D3**: `sqlite-vec` over Qdrant / lancedb. One file, no daemon,
  cross-platform.
- **D11**: Skills are filesystem-canonical; the DB is just an index
  cache. You can `git diff` your skills.
- **D12** (revised 2026-05-20): EmbeddingGemma-300M, not Qwen3.
  300 MB > 1.2 GB / 800 MB > 3 GB RAM is the right trade-off for a
  personal-assistant workload.
- **D14**: MCP tool permissions live in `mcp-servers.toml` — pro-server
  defaults plus pro-tool overrides. Compact for the common case.
