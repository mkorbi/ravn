---
title: Configure
description: Skills, MCP servers, and semantic memory.
sidebar:
  order: 2
---

Once ravn boots, three optional bits of configuration turn it from
"chat with an LLM" into "personal assistant with persistent context
and tool access".

## Skills (`~/.ravn/skills/`)

Skills are bundles of instructions plus optional scripts. The
filesystem is canonical (D11); ravn mirrors them into a SQLite index
for FTS5 + semantic search.

Three skills ship with the repo:

```bash
# macOS
cp -R crates/skills/initial/* ~/Library/Application\ Support/ravn/skills/

# Linux (XDG)
cp -R crates/skills/initial/* "${XDG_DATA_HOME:-$HOME/.local/share}/ravn/skills/"
```

On next startup, `ravn.log` shows:

```
INFO skills sync done inserted=3 updated=0 unchanged=0 deleted=0
```

A custom skill is just a directory with a `SKILL.md`:

```markdown
---
name: my-skill
description: |
  One paragraph describing when the LLM should reach for this.
trigger_patterns: ["keyword", "another keyword"]
allowed_tools: [shell, file_read]
---

# My Skill

## When to use
...

## Workflow
1. ...
2. ...
```

Re-syncs use a SHA-256 body hash — unchanged skills don't re-embed.
Skills you remove from disk get pruned from the DB on next start.

## MCP servers (`~/.ravn/mcp-servers.toml`)

Any [Model Context Protocol](https://modelcontextprotocol.io) server
runs as a subprocess; its tools register namespaced as
`<server>__<tool>` into the same registry as native tools.

```toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/projects"]
env = ["PATH", "HOME"]
permission = "write"          # server-wide default

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = ["PATH", "HOME", "GITHUB_PERSONAL_ACCESS_TOKEN"]
permission = "read"

# Per-tool override (rare).
[tools."github__create_issue"]
permission = "write"
```

The `env` field is a **whitelist** — anything not listed gets stripped
before the subprocess starts. Default (no `env`) forwards `PATH` and
`HOME` only.

Permission rules:

- `read` — tool runs silently.
- `write` / `exec` — TUI shows an approval modal; `y` allows once,
  `a` allows always (persisted across sessions, see [User guide](/ravn/user-guide/approvals/)).
- Per-tool entries beat per-server entries; missing entries default to `write`.

## Semantic memory (optional)

Three Markdown files in the data dir become part of every prompt:

| File | Purpose | Token cap |
|---|---|---|
| `soul.md` | Persona / identity | 800 |
| `memory.md` | Long-term facts ravn has learned | absorbs the rest |
| `user.md` | Model of the user (preferences, role, language) | 500 |

Total cap is 3000 tokens. Oversized files are truncated with a
trailing `…[truncated]` marker; `ravn.log` warns when truncation
happens.

The `memory_save` tool (write-permission) writes into these files
under a `## <section>` heading, defaulting to today's date.

## Embedding model

First time a message gets embedded, ravn downloads
[`onnx-community/embeddinggemma-300m-ONNX`](https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX)
(~300 MB) to the local fastembed cache. Subsequent runs reuse it.
Set `HF_HOME` if you want a custom cache location.
