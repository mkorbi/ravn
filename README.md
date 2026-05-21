# ravn

A personal-assistant AI agent in Rust. Local-first, MCP-native, skill-driven.

```
                                                  ,::::.._
                                               ,':::::::::.
                                           _,-'`:::,::(o)::`-,.._
                                        _.', ', `:::::::::;'-..__`.
                                   _.-'' ' ,' ,' ,\:::,'::-`'''
                               _.-'' , ' , ,'  ' ,' `:::/
                         _..-'' , ' , ' ,' , ,' ',' '/::
                 _...:::'`-..'_, ' , ,'  , ' ,'' , ,'::|
              _`.:::::,':::::,'::`-:..'_',_'_,'..-'::,'|
      _..-:::'::,':::::::,':::,':,'::,':::,'::::::,':::;
        `':,'::::::,:,':::::::::::::::::':::,'::_:::,'/
        __..:'::,':::::::--''' `-:,':,':::'::-' ,':::/
   _.::::::,:::.-''-`-`..'_,'. ,',  , ' , ,'  ', `','
 ,::SSt:''''`                 \:. . ,' '  ,',' '_,'
                               ``::._,'_'_,',.-'
                                   \\ \\
                                    \\_\\
                                     \\`-`.-'_
                                  .`-.\\__`. ``
                                     ``-.-._
                                         `
```

**Status:** Phase 2 complete (MCP + Skills + Hybrid Search). Phase 3 in progress (Reasoning-Router + Subagents).

## What is ravn?

A terminal-first AI agent that runs a hand-written ReAct loop on top of [`rig-core`](https://github.com/0xPlaygrounds/rig), with:

- **9 native tools** (file/shell/web/memory/datetime/session-search/skills) plus any [MCP server](https://modelcontextprotocol.io) you configure.
- **Three-tier permissions** (Read / Write / Exec) with an inline approval modal — `y` to allow once, `a` for a persistent per-tool allowlist.
- **Hybrid memory** — Markdown files (soul/memory/user) for stable identity, SQLite + FTS5 + sqlite-vec for episodic recall with semantic search.
- **Skills** as filesystem-canonical `SKILL.md` bundles with progressive disclosure.
- **Cache-aware** — Anthropic prompt caching on by default; live hit-rate in the status bar; warn under 60 %.
- **No daemons** — one SQLite file, one binary, runs on your laptop.

Built as a Rust Cargo workspace; see [`docs/`](https://mkorbi.github.io/ravn/) for the architecture overview.

## Quick start

```bash
# Build
git clone https://github.com/mkorbi/ravn.git
cd ravn
cargo build --release -p ravn-cli

# Run
export ANTHROPIC_API_KEY=sk-ant-…   # or OPENAI_API_KEY
./target/release/ravn
```

Type a message to chat, or one of these slash-commands:

| Command | Alias | Action |
|---|---|---|
| `/help` | `/h`, `/?` | List slash-commands |
| `/about` | | Reprint the startup splash |
| `/clear` | `/cls` | Wipe the scrollback |
| `/quit` | `/exit`, `/q` | Close ravn |

See the [Getting Started guide](https://mkorbi.github.io/ravn/getting-started/install/) for configuration (MCP servers, skills, semantic memory).

## Workspace layout

```
crates/
├── core          # ReAct loop + Budget + EventSink
├── llm           # LlmProvider trait + OpenAI/Anthropic adapters
├── tools         # Tool trait + Permission + Approver + 9 native tools
├── memory        # soul.md / memory.md / user.md + token-budget enforcement
├── embeddings    # fastembed-rs + EmbeddingGemma-300M (ONNX)
├── persistence   # sqlx + rusqlite + sqlite-vec; sessions/messages/events/skills
├── mcp           # rmcp client + mcp-servers.toml + McpToolAdapter
├── skills        # SKILL.md parser + filesystem→DB sync
└── cli           # ratatui TUI, slash-commands, approver
```

## Project documents

- [`PLAN.md`](PLAN.md) — phase-by-phase task list and decisions log (D1–D14).
- [`project.md`](project.md) — original architecture and implementation guide.
- [`docs/`](docs/) — user-facing documentation site (Astro Starlight).

## Acceptance status

| Phase | Scope | Status |
|---|---|---|
| 0 | Foundation (workspace, LlmProvider, SQLite schema, TUI) | ✅ |
| 1 | MVP (ReAct, 7 native tools, semantic memory, approval modal) | ✅ |
| 2 | MCP + Skills + Hybrid Search + Allowlist persistence | ✅ |
| 3 | Reasoning-Router + Subagents + StateGraph + Eval | 🚧 |
| 4 | Computer Use + Multi-Channel + Tauri | ⏳ |
| 5 | Eigener MCP-Server + A2A | ⏳ |
| 6 | RL & Self-Improvement | ⏳ |
| 7 | Polish & Public Release | ⏳ |

87 unit tests workspace-wide as of Phase 2. Smoketests in [`test.md`](test.md).

## License

[MIT](LICENSE) © Max Körbächer
