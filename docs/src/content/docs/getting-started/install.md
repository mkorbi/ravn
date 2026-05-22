---
title: Install
description: Build ravn from source and start your first session.
sidebar:
  order: 1
---

ravn is a Cargo workspace — clone, build, run. The CLI talks to
Anthropic or OpenAI and runs a full ReAct agent: native + MCP tools,
skills, hybrid memory, a reasoning router, and scheduled heartbeats.

## Prerequisites

- **Rust ≥ 1.91** (we pin MSRV to 1.91 because some transitive deps need it).
- **macOS or Linux.** Windows works for most things but the TUI uses
  `crossterm` with an alternate-screen buffer; YMMV.
- **An LLM API key** — either `ANTHROPIC_API_KEY` (recommended) or
  `OPENAI_API_KEY`.

Optional:

- **Node.js + `npx`** if you want to load MCP servers — most public MCP
  servers are distributed as npm packages.
- **cmake + a C/C++ compiler** for voice input (`/voice`) — `whisper-rs`
  builds whisper.cpp from source. On macOS: `brew install cmake` (Xcode CLT
  provides clang).
- A modern terminal that handles ANSI colors and Unicode.

## Build from source

```bash
git clone https://github.com/mkorbi/ravn.git
cd ravn
cargo build --release -p ravn-cli
```

The release binary lands at `target/release/ravn`. First build pulls
a few hundred crates; expect ~5 minutes on a modern laptop.

## First run

```bash
export ANTHROPIC_API_KEY=sk-ant-…
./target/release/ravn
```

You should see:

- The TUI in an alternate-screen buffer.
- A splash block (ASCII raven, version, slash-commands) as the first
  scrollback entry.
- A status line at the bottom with session id, token counts and the
  live USD spend.

Type a message and hit Enter. The model streams its response into the
scrollback. `/help` lists the slash-commands.

## Where ravn stores things

| Path | What lives there |
|---|---|
| `~/Library/Application Support/ravn/` (macOS) | data directory |
| `$XDG_DATA_HOME/ravn/` (Linux) | data directory |
| `state.db` | sessions, messages, events, FTS5, sqlite-vec |
| `soul.md` / `memory.md` / `user.md` | semantic memory (optional) |
| `skills/<name>/SKILL.md` | progressive-disclosure skills |
| `mcp-servers.toml` | MCP server config |
| `heartbeats.toml` | cron-scheduled heartbeat jobs |
| `whisper/ggml-base.bin` | local Whisper model for voice input (auto-downloaded) |
| `ravn.log` | tracing log (TUI sends `tracing` here, not to stderr) |

## Environment variables

| Var | Purpose |
|---|---|
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` | provider credential — one is required |
| `RAVN_MODEL` | override the default model (e.g. `claude-opus-4-7`, `gpt-4o`) |
| `OPENAI_BASE_URL` | point the OpenAI client at an OpenAI-compatible endpoint |
| `RUST_LOG` | tracing filter, e.g. `ravn=debug` |
| `HF_HOME` | cache dir for the downloaded embedding model |
| `RAVN_WHISPER_MODEL` | path to a ggml Whisper model (overrides the auto-download) |
| `RAVN_VOICE_LANG` | transcription language hint, e.g. `en`; default auto-detect |

## Next steps

- [Configure](/ravn/getting-started/configure/) MCP servers and skills.
- [User guide](/ravn/user-guide/tui/) — keybindings, slash-commands, approval flow.
