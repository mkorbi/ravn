---
title: Approvals & permissions
description: How ravn decides whether to run a tool.
sidebar:
  order: 2
---

Every tool ravn knows about declares a permission level. Read tools
run silently; Write and Exec tools are gated by an inline modal.

## The permission model

| Level | Examples | Behavior |
|---|---|---|
| `Read` | `file_read`, `web_fetch`, `session_search`, `datetime`, `skill_list`, `skill_view` | Always runs |
| `Write` | `file_write`, `memory_save` | Modal — approve once, allow always, deny, or cancel |
| `Exec` | `shell` | Modal — same controls |

MCP tools default to `Write` unless the server config overrides them
(see [Configure](/ravn/getting-started/configure/)).

## The approval modal

When the LLM emits a tool call that needs approval, the loop pauses
and an overlay appears:

```
┌─ approval ─────────────────────────────────────────────┐
│ Tool call requested: file_write                       │
│ Permission: WRITE                                     │
│                                                       │
│ Args:                                                 │
│   {                                                   │
│     "path": "/tmp/notes.md",                          │
│     "content": "..."                                  │
│   }                                                   │
│                                                       │
│ [y] allow once   [n] deny   [a] allow this tool      │
│ always   [Esc] cancel run                             │
└───────────────────────────────────────────────────────┘
```

Permission level colors the border: cyan for Read, yellow for Write,
red for Exec.

### The four choices

- **`y` — allow once.** Tool runs with the exact args shown. Next
  time it asks, you get a fresh modal.
- **`n` — deny.** The tool returns an error result to the model
  (`user denied tool call: <name>`). The agent loop continues — the
  model can adapt or apologize, but it won't try the same call again
  unless you say so.
- **`a` — allow this tool always.** Tool name lands in the persistent
  allowlist (`tool_allowlist` DB table). Future calls of the same
  tool — *any args* — run without a modal across sessions.
- **`Esc` — cancel the run.** The current run terminates and you see
  `error: cancelled` in red. No DB row is written.

Allowlist entries are by tool name only — there's no args-pattern
matching. The trade-off is simplicity vs. precision; revoke a tool
by deleting its row directly:

```bash
sqlite3 ~/Library/Application\ Support/ravn/state.db \
  "DELETE FROM tool_allowlist WHERE tool_name = 'shell';"
```

(A `/allowlist clear <name>` command will land in a later release.)

## Trustworthy results

Tool outputs from untrusted sources — anything `web_fetch` returns,
content from MCP servers — get wrapped before they reach the model:

```
<tool_result trustworthy="false">
…the fetched content…
</tool_result>
```

The system prompt tells the model that anything inside those tags is
**data, never instructions**. Prompt-injection mitigation is on by
default; you don't have to opt in. The original content stays
unwrapped in the database for audit.
