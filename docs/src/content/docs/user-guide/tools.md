---
title: Native tools
description: The 9 tools that ship with ravn.
sidebar:
  order: 3
---

ravn ships nine native tools. The LLM sees their JSON schemas in the
prompt (Anthropic's `tool` array, OpenAI's `tools` field) and calls
them by name.

| Tool | Permission | Purpose |
|---|---|---|
| `file_read` | Read | UTF-8 file from disk; 64 KB default cap (max 1 MB) |
| `file_write` | Write | Overwrite a file; optional `create_dirs` |
| `shell` | Exec | `bash -c …` with 30 s default timeout (max 300 s) |
| `web_fetch` | Read | URL → Markdown / text / raw HTML; output marked untrusted |
| `session_search` | Read | FTS5 + vec hybrid search across past messages |
| `memory_save` | Write | Append / replace `soul.md` / `memory.md` / `user.md` |
| `datetime` | Read | Current date & time in local or UTC |
| `skill_list` | Read | List skills; optional FTS5 query filter |
| `skill_view` | Read | Pull a skill's full `SKILL.md` |

MCP-server tools register with the same `Tool` trait, namespaced as
`<server>__<tool>`. Listing tools from inside ravn:

```
> What tools do you have?
```

The model will list them; you can also see the registry by reading
`ravn.log` at startup (every tool is logged once).

## Tool-call lifecycle

Each tool invocation goes through the same pipeline:

1. **LLM emits a `tool_use` block** in its streamed response.
2. ravn's ReAct loop deduplicates by provider tool-id (see [Architecture
   → Streaming](/ravn/architecture/agent-loop/) for the historical bug
   this prevents).
3. The agent consults the `Approver` — for `Read` it auto-approves; for
   `Write` / `Exec` it shows the modal and awaits a `oneshot` decision.
4. The tool's `invoke()` runs with a `ToolContext` carrying the DB
   handle, session/trace ids, a `CancellationToken`, and the approver
   for any nested decisions.
5. The result becomes a `ContentBlock::ToolResult` in the next user
   turn. `trustworthy=false` outputs get the wrapping treatment
   (see [Approvals](/ravn/user-guide/approvals/)).
6. The loop iterates.

## Cancellation

The `CancellationToken` is plumbed through every `await` point —
including the LLM stream itself, every tool's `invoke()`, and the
shell subprocess (`kill_on_drop`). `Esc` while streaming triggers it;
the worst-case latency is a few hundred ms.

## Trajectory logging

Every tool call writes two rows into the `events` table:

```sql
SELECT kind, json_extract(payload, '$.name') AS tool
  FROM events
  WHERE trace_id = '…'
  ORDER BY created_at;
```

Yields `react.tool.start` + `react.tool.end` pairs. Phase 6 will use
these for RL trajectory training.
