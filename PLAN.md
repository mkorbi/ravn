# Projektplan: Agentisches KI-System in Rust

> Begleitdokument zu [project.md](./project.md). Konvertiert die dortige 7-Phasen-Roadmap in eine konkrete Task-Liste mit Milestones, Akzeptanzkriterien und Abhängigkeiten.

## Leitprinzipien (aus project.md)

1. **Simplest solution first** — ReAct nackt, ohne Framework, bevor komplexere Patterns adoptiert werden.
2. **Cache-stabile Prompt-Reihenfolge** ab Tag 1: Tools → System → Skills-Meta → MEMORY.md → SOUL.md → History → User-Turn.
3. **Trajectory-Logging ab Tag 1** — sonst ist späteres RL unmöglich.
4. **Single-Agent als Default**, Multi-Agent nur bei read-heavy Parallel-Recherche (15× Token-Cost).
5. **Native für High-Frequency, MCP für High-Variety**.
6. **Versionen pinnen** — `rig` und `rmcp` sind pre-1.0 mit Breaking Changes.

---

## Geklärte Architektur-Entscheidungen

| # | Entscheidung | Pick | Konsequenz |
|---|---|---|---|
| D1 | Agent-Abstraktion | **`rig-core` 0.37+ für Provider, ReAct-Loop selbst geschrieben** | rig liefert OpenAI/Anthropic/Ollama/... als Provider-Backend; Loop/Tool-Calling/State-Machine bleibt unter unserer Kontrolle. Versionen pinnen (pre-1.0). |
| D2 | SQLite-Treiber | **Hybrid: `sqlx` 0.8+ default, `rusqlite` 0.32+ in `spawn_blocking` für FTS5/vec0** | Normale CRUD via sqlx (compile-time-checked); Custom-FTS5-Tokenizer und `sqlite-vec` `vec0`-Tables via rusqlite, wo sqlx-Support fehlt. Beide DB-Deps im `persistence`-Crate. |
| D3 | Vector-Store | **`sqlite-vec` 0.1** | Single-File, läuft im selben SQLite wie Sessions. Pre-v1 — `VectorStore`-Trait anlegen, damit Migration zu lancedb später möglich ist (kein vorzeitiges Trait-Engineering, aber Naming/Boundary sauber halten). |
| D4 | Desktop-UI | **`ratatui` ab Phase 0 + `tauri` 2.0 parallel ab Phase 4** | TUI bleibt Dev-Werkzeug und Power-User-Interface. Tauri-App ab Multi-Channel-Phase für End-User-Demos. UI teilt sich Gateway/WebSocket-Backend (Phase 2+). |
| D5 | Lokale Inferenz | **Erst ab Phase 3** | `mistral.rs` 0.7.x kommt mit Reasoning-Router (Phase 3.1). Phase 0–2: Cloud-only (Anthropic+OpenAI). |
| D6 | MSRV | **Rust 1.91+ pinnen** | `rust-version = "1.91"` im Workspace. Begründung: transitiv via aws-smithy bei `lancedb` ≥0.20 (auch wenn lancedb nicht initialer Default ist, lassen wir den Pfad offen). |
| D7 | Approval-UX (Phase 1) | **Inline-Modal y/n/a im TUI** | Bei Write/Exec-Tool pausiert der Loop, Overlay zeigt Tool-Name+Args. `y`=ja-diesmal, `n`=nein, `a`=Allowlist-Pattern fuer kuenftige Auto-Allow. `Esc` cancelt den Run. |
| D8 | Memory-Crate (Phase 1) | **Neuer `crates/memory`** | Eigener Crate fuer Working/Episodic/Semantic/Procedural Memory (project.md §1.4). Phase 1 implementiert nur Semantic (Markdown-Files); Episodic kommt in Phase 2. |
| D9 | Web-Tools (Phase 1) | **`web_fetch` only** | reqwest + HTML-to-Markdown fuer URL→Markdown, kein externer API-Key noetig. `web_search` verschiebt sich auf Phase 2 (ggf. via MCP-Server statt nativer Implementation). |
| D10 | Cache-Tracking (Phase 1) | **`hit_rate` in Statuszeile** | `hit_rate = cache_read / (input + cache_read + cache_creation)` als 4. Wert in der TUI-Statusbar. PLAN.md Trigger `<60%` loest Warnung aus. |
| D11 | Skills-Storage (Phase 2) | **Hybrid: Filesystem canonical + DB-Spiegel für Index** | `~/.ravn/skills/<name>/SKILL.md` + `scripts/` + `reference/` als Single Source of Truth (git-versionbar, im Editor). SQLite spiegelt Frontmatter + Body für FTS5 + Vector-Index. Sync bei Skill-Load + manueller `skills::reload`. Mehr Code als FS-only, aber beste Such-UX. |
| D12 | Embedding-Modell (Phase 2) | **EmbeddingGemma-300M (768 dim, multilingual)** | `onnx-community/embeddinggemma-300m-ONNX` via fastembed-rs (ONNX-Runtime). ~300 MB Modell, multilingual (DE/EN), spürbar schneller als Qwen3-Embedding-0.6B (~10-30× kleinerer Footprint). `messages_vec` + `skills_vec` haben `vec0(embedding float[768])`. **D12-Revision** vom 2026-05-20: ursprünglich Qwen3 0.6B gewählt, aber ~1.2 GB Download + ~3 GB RAM zu schwer für den Personal-Assistant-Use-Case; EmbeddingGemma reicht für Session-Search + Skill-Matching. |
| D13 | Allowlist-Persistence (Phase 2) | **`tool_allowlist(tool_name PRIMARY KEY, created_at)`** | DB-Table macht `a`-Taste im Modal session-übergreifend. Pure Name-Match — kein Args-Pattern. Risiko: einmal erlaubt = alle künftigen Args ohne Modal. Mitigation: User kann via `/allowlist clear <name>` revoken (Phase 2 Followup). |
| D14 | MCP-Tool-Permissions (Phase 2) | **Pro-Server-Default in `mcp-servers.toml` + Pro-Tool-Override** | `[servers.github] permission = "read"` setzt Default für alle Tools dieses Servers. `[tools."github__create_issue"] permission = "write"` überschreibt einzelne. Wenn kein Server-Default → conservative Default `write` (mit Modal). |
| D15 | Reasoning-Router (Phase 3) | **Pure Heuristik** | step_depth > 3, last tool returned `is_error`, optional user-feedback-signal → Deep. Keine extra LLM-Calls, deterministisch, leicht zu debuggen. Klassifikator-LLM kommt in Phase 6 mit RL-Training. |
| D16 | Default-Models (Phase 3) | **Fast = Sonnet 4.6, Deep = Opus 4.7 + Extended Thinking** | Anthropic-only Default. Sonnet als Workhorse (Cache-optimiert via `CacheMode::Auto`), Opus mit `thinking.budget_tokens` für harte Probleme. Cross-Provider-Mix (OpenAI o3) bleibt opt-in via `RAVN_MODEL` env. |
| D17 | Subagents (Phase 3) | **In Phase 3 inkludieren mit harter Nested-Sperre** | `subagent::delegate()` spawned `tokio::task` mit read-only-Tool-Subset + isoliertem `RunContext` + eigenem `Budget`. Hart-Verbot von Nested-Subagents: das Read-Only-Tool-Set enthält keine `delegate`-Funktion. Subagent gibt nur `summary` + `tokens_used` + `artifacts` zurück, nie Roh-Context. |
| D18 | Eval-Set (Phase 3) | **30 hand-crafted Tasks + LLM-as-Judge (Sonnet)** | PLAN.md ursprüngliche „≥500 Trajectories"-Voraussetzung ist nicht erfüllt; statt skip schreiben wir 30 repräsentative Tasks mit Ground-Truth-Annotation (Format: input + expected-outcome-rubric). Sonnet 4.6 als Judge mit strukturiertem JSON-Output. Phase-6-RL-Daten kommen on top wenn echte Nutzung da ist. |

---

## Phase 0 – Foundation (Woche 1–2)

**Goal**: Kompilierfähiger Workspace, der einen einzelnen Anthropic/OpenAI-Call mit Streaming in eine TUI rendert, Kosten misst, jede Iteration in SQLite ablegt.

### Tasks
- [ ] **0.1** Cargo-Workspace anlegen mit `resolver = "2"`, `rust-version`, Workspace-Dependencies.
- [ ] **0.2** Crates `core`, `llm`, `persistence`, `cli` skeletten + leere `lib.rs`.
- [ ] **0.3** `LlmProvider`-Trait (siehe project.md §1.1) mit `complete`/`stream`/`supports_caching`/`supports_reasoning`.
- [ ] **0.4** `llm::openai` Adapter — wrappt `rig::providers::openai::Client` (D1). Übersetzt unsere `CompletionRequest`/`Response`/`Message` ↔ rig's Typen.
- [ ] **0.5** `llm::anthropic` Adapter — wrappt `rig::providers::anthropic::Client` (D1). `cache_control`-Marker via `CompletionRequest::additional_params`, falls rig's native API es nicht direkt exposed.
- [ ] **0.6** SQLite-Schema-Migration: `sessions`, `messages`, `events`, `messages_fts` (FTS5). Migration via `sqlx::migrate!` oder `rusqlite_migration`.
- [ ] **0.7** `persistence::repo` mit Session-Open/Close, Message-Append, Event-Append.
- [ ] **0.8** ratatui-TUI: scrollbares Message-Pane + Input-Pane + Streaming via `tokio::sync::mpsc`.
- [ ] **0.9** `tracing` + `tracing-subscriber` mit JSON-Layer für Persistenz, Pretty-Layer für TTY.
- [ ] **0.10** Cost-Tracking: Token-Counts aus Response-Usage extrahieren → `sessions.cost_usd` mit Pricing-Tabelle (hardcoded YAML).
- [ ] **0.11** Cache-stabile Prompt-Reihenfolge als Konvention in `llm::caching` durchsetzen.

### Akzeptanzkriterien
- `cargo run -p agent-cli` startet TUI, akzeptiert Input, streamt LLM-Response.
- Jede Session erzeugt ≥1 Row in `sessions`, alle Messages in `messages`.
- `events`-Tabelle enthält ≥1 `trace_id`-Event pro LLM-Call.
- Cost-USD wird pro Session aggregiert und in der TUI-Statuszeile angezeigt.
- Zweiter Run mit identischem System-Prompt zeigt `cache_read_input_tokens > 0` (Anthropic).

---

## Phase 1 – MVP (Woche 3–6)

**Goal**: Lauffähiger ReAct-Agent mit nativen Tools, Markdown-Memory, Approval-Gate für Writes/Exec, sauberer Cancellation.

**Abhängigkeit**: Phase 0 abgeschlossen. Decisions [D7–D10] geklärt.

### Tasks (Ausfuehrungs-Reihenfolge)
- [ ] **1.3** `crates/tools` skeleton mit `Tool`-Trait inkl. `permission(): Read|Write|Exec` (project.md §1.11). Schema-Generation via `schemars`.
- [ ] **1.6** Neuer `crates/memory` ([D8]). Loader fuer `~/.ravn/{soul.md,memory.md,user.md}`. Working-/Episodic-Stubs leer (Phase 2).
- [ ] **1.7** Hard-Limits: Memory-Total ≤ 3000 Tokens, Soul ≤ 800, User ≤ 500. Truncation mit Warning.
- [ ] **1.1** `core::loop` ReAct-Implementation (thought → action → observation) mit Hard-Cap 50 Steps + Token-Budget. Integriert llm + tools + memory.
- [ ] **1.2** `core::budget` (Step/Token/Cost-Limits) + `tokio::sync::CancellationToken` durch gesamten Loop.
- [ ] **1.4** Native Tools (7, [D9] ohne `web_search`): `file_read`, `file_write` (Write), `shell` (Exec), `web_fetch` (Read, via reqwest+html2md), `session_search` (Read, nutzt `ravn_persistence::messages::search`), `memory_save` (Write, MEMORY.md/USER.md append-or-update), `datetime` (Read).
- [ ] **1.5** Tool-Schemas an llm-Provider serialisieren via `ToolSchema` (existing in `ravn_llm`).
- [ ] **1.9** Approval-UI in TUI ([D7]): Inline-Modal mit `y`/`n`/`a` (Allowlist). `Esc` cancelt den Run. Allowlist persistiert in DB.
- [ ] **1.10** Tool-Output-Wrapping in `<tool_result trustworthy="false">…</tool_result>` fuer Prompt-Injection-Mitigation (untrusted outputs).
- [ ] **1.8** Cache-Hit-Rate-Tracking ([D10]): pro Session aggregieren, in Statuszeile als 4. Wert anzeigen, `<60%` triggert Warn-Log.
- [ ] **1.11** Acceptance-Smoketest end-to-end (user-verifiziert — siehe Checkliste unten).

### Phase 1 Smoketest-Checkliste

Vor dem Test:
```bash
export ANTHROPIC_API_KEY=sk-ant-…
DB=~/Library/Application\ Support/ravn/state.db    # macOS path
cargo run --release -p ravn-cli
```

Nutze ein zweites Terminal für SQLite-Verifikation. Markiere bestandene Punkte mit `[x]`.

**A — Startup & UI**
- [X] TUI öffnet (alternate screen), Scrollback leer, Statuszeile zeigt `session <id> │ in 0 out 0 cache_r 0 hit  -- │ $0.0000`
- [X] Tippen erscheint live in Input-Pane

**B — Plain Chat (Regression aus Phase 0)**
- [X] Eingabe `hi` + Enter → Antwort streamt in
- [X] Cursor `▌` während streaming sichtbar, verschwindet bei Done
- [X] Statuszeile zeigt nach Antwort: `in > 0`, `out > 0`, `$` mit positivem Betrag

**C — Read-Tool ohne Approval** (`datetime`)
- [ ] Eingabe: `What is today's date in Berlin?` → Assistant ruft `datetime` (siehe dim Zeile `🔎 datetime {…}` im Scrollback), KEIN Modal
- [ ] Ergebnis-Zeile `  ✓ datetime: 2026-…` erscheint
- [ ] Endgültige Antwort enthält aktuelles Datum
Test dailed with: error: llm: provider anthropic returned 400: ProviderError: SSE Error: Invalid status code│
│400 Bad Request with message:                                                             │
│{"type":"error","error":{"type":"invalid_request_error","message":"messages.3.content.1:  │
│`tool_use` ids must be unique"},"request_id":"req_011Cb4iHgzXnEXroppGnqxBE"}
But then after the next input it shows datetime correctly

**D — Write-Tool mit Approval-Modal** (`file_write`)
- [X] Eingabe: `Write the word "test" to /tmp/ravn_test.txt`
- [X] Modal erscheint, zentriert, mit Tool=`file_write`, Permission=`WRITE` (gelb), Args pretty-printed, Hint-Zeile
- [X] Eingabe ohne Modal blockiert (`> `-Prompt zeigt `(approval needed)`)
- [X] `y` → Modal verschwindet, dim Zeile `✓ file_write: wrote 4 bytes …`
- [X] `cat /tmp/ravn_test.txt` → `test`
I get an error: error: llm: provider anthropic returned 400: ProviderError: SSE Error: Invalid status code

**E — Modal denial**
- [X] Wieder: `Write "abc" to /tmp/ravn_deny.txt`
- [X] Modal → `n` → dim Zeile `denied: file_write` (gelb)
- [X] Datei `/tmp/ravn_deny.txt` existiert **nicht**
- [X] Assistant-Antwort sollte erkennen, dass das Tool verweigert wurde

**F — Modal cancel mit Esc**
- [X] Eingabe: `Write "x" to /tmp/ravn_esc.txt`
- [X] Modal → `Esc` → ganzer Run bricht ab, `error: cancelled` (rot) im Scrollback
- [X] Datei `/tmp/ravn_esc.txt` existiert **nicht**

**G — Exec-Tool mit Approval** (`shell`)
- [x] Eingabe: `Run "echo hello world" via shell`
- [x] Modal mit Permission=`EXEC` (rot)
- [x] `y` → Tool läuft, dim Zeile zeigt `✓ shell: exit=0` (excerpt)
- [x] Assistant repräsentiert das Output korrekt

**H — Allowlist (`a`-Taste)**
- [x] Eingabe: `Run "uname -s" via shell`
- [x] Modal → `a` → Tool läuft
- [x] Direkt danach: `Run "whoami" via shell` → **kein** Modal, Tool läuft direkt
- [x] Allowlist gilt nur in dieser Session (neu starten → wieder Modal)

Error, for some reason every input looks like is running twice, also the apporval via modal winfows: you:                                                                                      │
│Run "whoami" via shell                                                                    │
│                                                                                          │
│ravn:                                                                                     │
│Sure!                                                                                     │
│                                                                                          │
│⚙ shell {"command":"whoami"}                                                              │
│                                                                                          │
│  ✓ shell: exit=0                                                                         │
│                                                                                          │
│⚙ shell {"command":"whoami"}                                                              │
│                                                                                          │
│  ✓ shell: exit=0

**I — Esc cancel während streaming**
- [X] Eingabe: `Write me a 500-word essay about Rust ownership`
- [X] Sobald Tokens reinkommen → `Esc` → Loop bricht ab in <1s
- [X] Statuszeile: keine weitere Token-Erhöhung

**J — Untrusted-Source wrap** (`web_fetch`)
- [X] Eingabe: `Fetch https://example.com and tell me what it says`
- [X] `web_fetch` läuft (kein Modal — Read-Permission)
- [X] Assistant sollte sich darauf beziehen, dass der Inhalt aus externer Quelle stammt
- [ ] Verifikation: `sqlite3 "$DB" "SELECT content FROM messages WHERE role='user' ORDER BY id DESC LIMIT 1;"` → enthält `<tool_result trustworthy="false">`
Return: ravn:                                                                                     │
│It looks like you're running a SQLite query to fetch the most recent user message from a  │
│`messages` table. Want me to run that for you? If so, I'd need to know:                   │
│                                                                                          │
│1. **The path to your database file** — what should `$DB` be?                             │
│                                                                                          │
│Or if you're just sharing the command for reference, what are you trying to accomplish?   │
│I'm happy to help with:                                                                   │
│                                                                                          │
│- **Running the query** against a specific DB file

**K — Multi-Step Task** (das Big-Acceptance-Item aus PLAN.md)
- [X] Eingabe: `Fetch https://example.com and save the page title to /tmp/ravn_title.txt`
- [X] Erwarteter Toolchain: `web_fetch` → (Approval-Modal für) `file_write` → final
- [ ] `cat /tmp/ravn_title.txt` enthält `Example Domain` o.ä.
error:  ✗ file_read: io: /tmp/ravn_title.txt: No such file or directory (os error 2)

**L — Persistence-Verifikation**
```bash
sqlite3 "$DB" <<SQL
SELECT id, channel, model, input_tokens, output_tokens, cost_usd FROM sessions ORDER BY started_at DESC LIMIT 3;
SELECT COUNT(*) AS msg_count, session_id FROM messages GROUP BY session_id ORDER BY msg_count DESC LIMIT 3;
SELECT kind, COUNT(*) FROM events GROUP BY kind ORDER BY COUNT(*) DESC;
SQL
```
- [ ] `sessions`: mind. 1 Row mit nicht-null `model`, positivem `cost_usd`
- [ ] `messages`: mehrere Rows pro Session (user + assistant + tool_result-bearing user)
- [ ] `events`: `react.tool.start`, `react.tool.end`, `react.done`, `llm.request`/`llm.response` falls noch vorhanden

**M — Cache-Hit-Rate ≥ 60%** (PLAN.md Threshold)
- [x] **Erste Session beenden** (Ctrl-C bei leerem Input → quit)
- [X] **Neue Session starten**: `cargo run --release -p ravn-cli`
- [X] Genau **dieselbe** erste User-Eingabe wie in vorheriger Session
- [X] Nach Antwort: Statuszeile `cache_r > 0`, `hit XX%` mit `XX ≥ 60`
- [ ] Bei `< 60%`: → in `~/Library/Application Support/ravn/ravn.log` sollte `WARN … cache hit-rate below 60%` stehen

**N — Memory-Loader** (Phase 1.6/1.7)
- [X] Beende die TUI
- [X] Schreibe Test-Memory:
  ```bash
  mkdir -p ~/Library/Application\ Support/ravn
  echo "Max prefers German for explanations." > ~/Library/Application\ Support/ravn/user.md
  echo "I am ravn." > ~/Library/Application\ Support/ravn/soul.md
  ```
- [X] Starte TUI neu, frage: `What do you know about me?`
- [X] Antwort sollte auf German/Max referenzieren (Identifier aus user.md)
- [ ] Hard-Limits-Check: schreibe sehr lange user.md (`>2000 chars`), starte neu → `ravn.log` sollte `WARN … user.md truncated to 500-token cap` enthalten

**O — Budget-Cap**
- [X] In `crates/core/src/agent.rs::AgentConfig::new`, max_steps temporär auf 2 setzen (oder per env-var falls implementiert)
- [X] Eingabe die mehrere Tool-Schritte braucht: `Run "ls /" then "ls /tmp" then "ls /etc" via shell`
- [X] Loop terminiert mit `error: budget exceeded: max_steps` nach 2 Steps
- [X] AgentConfig wieder zurücksetzen
Error in test, you mean max_tokens?

### Akzeptanzkriterien (Pass-Fail Phase 1)

Phase 1 ist abgenommen wenn:
- A, B, C, D, E, F, G, H, I, J, K alle ✓ (UI/Tool-Flow)
- L ✓ (Persistence)
- M ✓ (Cache-Hit-Rate ≥ 60% auf wiederholtem Turn)
- N ✓ (Memory-Loader funktioniert)
- O ✓ (Budget-Cap funktioniert — optional manuell zu testen)

---

## Phase 2 – MCP + Skills (Woche 7–10)

**Goal**: Externe MCP-Server konsumierbar; Skill-Discovery via Progressive Disclosure; semantisches Session-Search.

**Abhängigkeit**: Phase 1 abgeschlossen. Decisions [D11–D14] geklärt.

### Tasks (Ausführungs-Reihenfolge)
- [ ] **2.11** Approval-Allowlist persistieren ([D13]): neue Migration `tool_allowlist(tool_name PK, created_at)`; `TuiApprover` lädt sie beim Start + schreibt bei `AllowAndRemember`. Kleiner Quick-Win, baut nichts Neues drumherum.
- [ ] **2.8** `fastembed-rs` 5.13+ in neuer `crates/embeddings`-Crate. Lazy-Load `Qwen3-Embedding-0.6B` ([D12]) beim ersten Embedding-Call. ONNX-Runtime via fastembed default.
- [ ] **2.9** `sqlite-vec` 0.1 als loadable Extension via `rusqlite::Connection::load_extension`. Neue Migration: `messages_vec(embedding float[1024])` + `skills_vec(embedding float[1024])`. Embedding-Pipeline batched (256 docs/call).
- [ ] **2.10** Hybrid Session-Search: `ravn_persistence::messages::search_hybrid(query, limit)` macht FTS5 + Vec parallel, mergt via Reciprocal Rank Fusion (RRF). `session_search`-Tool nutzt es ab jetzt.
- [ ] **2.1** `mcp::client` in neuer `crates/mcp`-Crate über `rmcp` 0.16+. stdio-Transport (Subprocess) + HTTP-Transport (für Cloud-MCPs). Wrappt MCP-Tools als `ravn_tools::Tool`-Trait-Impls.
- [ ] **2.2** MCP-Server-Konfig: `~/.ravn/mcp-servers.toml`. Schema: `[servers.<name>] command/args/env_whitelist/permission`; `[tools."<server>__<tool>"]` Override. Load + register beim Start.
- [ ] **2.3** Integration-Smoketest mit 3 öffentlichen Servern: `@modelcontextprotocol/server-filesystem`, `@modelcontextprotocol/server-github`, `@playwright/mcp`. Lokale subprocess-Tests, kein CI (Phase 3).
- [ ] **2.4** `crates/skills` Crate-Skelett. `SKILL.md`-Parser (YAML-Frontmatter via `serde_yaml`); Sync-Logik FS → SQLite-Spiegel ([D11]).
- [ ] **2.5** Skill-Registry: FTS5 + Vec im DB-Spiegel; Top-K-Match auf User-Prompt via Embedding der ersten User-Message. Builder gibt sortiertes `Vec<SkillMeta>` für PromptBuilder zurück.
- [ ] **2.6** Progressive Disclosure: `skill_list` (alle Skill-Metadaten, ~100 Tok/Skill) und `skill_view <name>` (lädt SKILL.md + ggf. referenzierte Files) als native Tools.
- [ ] **2.7** 3 Initial-Skills shippen unter `~/.ravn/skills/`: `git-workflow`, `web-research`, `note-taking`. (`code-review` + `daily-planning` als Phase-3-Stretch).

### Akzeptanzkriterien
- Agent kann externen MCP-Server (z.B. Playwright) ohne Code-Änderung via `mcp-servers.toml` adden.
- 100 fiktive Skills im Registry → Initial-Prompt-Overhead < 12k Tokens dank Progressive Disclosure.
- `session_search "topic"` liefert in <100 ms relevante Ergebnisse aus Vorsessions via FTS5+Vec-Hybrid.
- Allowlist persistiert über Sessions in DB (`tool_allowlist` table).
- Allowlist-Eintrag aus Session A wirkt in Session B beim selben Tool.

---

## Phase 3 – Subagents + Reasoning (Monat 3–4)

**Goal**: Reasoning-Router schaltet zwischen Fast/Deep Mode; isolated Subagents für Read-Heavy-Tasks; State-Machine mit Checkpointing.

**Abhängigkeit**: Phase 2 abgeschlossen; Trajectory-Logs ≥ 500 Tasks für Router-Training.

### Tasks
- [ ] **3.1** `core::router` Klassifikator: Heuristik first (Step-Depth, Tool-Output-Ambiguity, User-Feedback), kleines LLM als Fallback.
- [ ] **3.2** Reasoning-Mode-Enum (`Fast|Deep|Search|Plan|Reflect`), Mode-Dispatch im Loop.
- [ ] **3.3** OpenAI o-series Adapter (`reasoning_effort`, kein `temperature`).
- [ ] **3.4** Anthropic Extended Thinking Adapter (`thinking.budget_tokens`, Thinking-Blocks turn-übergreifend persistieren).
- [ ] **3.5** Reflexion-Retry Pattern: nach Failure Self-Critique + Re-Plan, max 3 Versuche.
- [ ] **3.6** `orchestration::graph`: typed `StateGraph<S>` mit Node-Trait, Edge-Dispatchern, Entry/END.
- [ ] **3.7** Checkpoint-Trait: pro Node-Transition Serialize via `postcard` in `events`. Resume aus letztem Checkpoint.
- [ ] **3.8** `orchestration::subagent::delegate()`: spawn `tokio::task` mit read-only Tool-Subset, isoliertem Context, eigenem Budget.
- [ ] **3.9** Subagent-Result: nur `summary`+`tokens_used`+`artifacts`, kein Roh-Context-Return.
- [ ] **3.10** Hartes Verbot von Nested Subagents (Compile-Time-Marker oder Runtime-Guard).
- [ ] **3.11** Eval-Set bauen: 30–50 reale Tasks aus Phase 1/2-Logs, LLM-as-Judge mit Rubrik.

### Akzeptanzkriterien
- Eval-Score Fast-Mode-only vs Hybrid-Router: ≥ 10 % Quality-Gain bei ≤ 3× Kosten.
- Crash mit `kill -9` → Resume aus Checkpoint funktioniert.
- Subagent-Task „find all callers of `foo`" terminiert mit Summary < 500 Tokens.

---

## Phase 4 – Computer Use + Multi-Channel + Tauri-Init (Monat 5–6)

**Goal**: A11y-Tree-First Desktop-Automation; Browser via CDP; Voice + Telegram als alternative Channels; proaktive Heartbeats; **erste Tauri-Desktop-App** für End-User-Demos.

**Abhängigkeit**: Phase 3 abgeschlossen. Voraussetzung für Tauri: Gateway/WebSocket aus Phase 2 stabil.

### Tasks — Computer Use
- [ ] **4.1** `computer_use::a11y::linux` via `atspi` 0.28+.
- [ ] **4.2** `computer_use::a11y::windows` via `uiautomation`.
- [ ] **4.3** `computer_use::a11y::macos` via `objc2` + AXUIElement (Risk: Swift-Bridge ggf. nötig — siehe Caveats).
- [ ] **4.4** Vision-Fallback: `xcap`-Screenshot, in Claude/GPT-Vision-Tool.
- [ ] **4.5** `computer_use::input` via `enigo` 0.6+ (cross-platform).
- [ ] **4.6** `computer_use::browser` via `chromiumoxide` 0.7+ (direct CDP).

### Tasks — Multi-Channel
- [x] **4.7** Voice-In: `whisper-rs` lokal **oder** OpenAI-Whisper-API. → `crates/voice` (cpal capture + local Whisper), `/voice` slash-command, transcript in den Input-Buffer; Modell lazy-download nach `~/.ravn/whisper/`.
- [ ] **4.8** Voice-Out: `piper-rs` lokal **oder** ElevenLabs HTTP.
- [ ] **4.9** Telegram-Bridge via `teloxide` — Pro-User-Session-Mapping.
- [x] **4.10** Heartbeat-Scheduler via `tokio-cron-scheduler` — User-definierte Trigger („jeden Morgen 8 Uhr Calendar-Sync"). → `crates/heartbeat` (`heartbeats.toml`, per-job Allowlist-Approver, `/heartbeat` slash-commands).
- [x] **4.11** Persistent World State: typed Rust-Struct (`Projects`, `OpenTabs`, `WatchTargets`), serialisiert in SQLite. → `ravn_persistence::world` + Prompt-Injection + `world_write`-Tool.

### Tasks — Tauri-Desktop (parallel zu Computer-Use)
- [ ] **4.12** `desktop`-Crate mit `tauri` 2.0 anlegen; Frontend in TypeScript+Vite (oder Dioxus-WebView wenn pure-Rust gewünscht — separat entscheiden).
- [ ] **4.13** Tauri-Frontend verbindet sich gegen lokales `axum`-WebSocket-Gateway (Phase 2.x) — kein duplicated State.
- [ ] **4.14** Message-Stream-Rendering, Tool-Call-Cards, Approval-Modals als UI-Komponenten.
- [ ] **4.15** Plattform-Build-Pipeline: `cargo tauri build` auf macOS/Linux/Windows in CI (unsigned MVP).
- [ ] **4.16** Feature-Parität mit TUI verifizieren (Session-Liste, Memory-Editor, Skill-Browser).

### Akzeptanzkriterien
- Computer-Use-Task „öffne $App, klicke Button X" funktioniert via A11y ohne Vision auf mind. 2 Plattformen.
- Token-Cost pro Computer-Use-Step < 2k Tokens (vs. Vision ~10–20k).
- Telegram-Message → Response zurück durchgängig.
- Heartbeat triggert geplante Action ohne User-Eingabe.
- Tauri-App startet auf macOS und Linux, lädt eine bestehende Session via Gateway, streamt eine LLM-Antwort.
- TUI und Tauri zeigen identische Daten (gleicher Gateway, gleiche DB).

---

## Phase 5 – Eigener MCP-Server + A2A (Monat 7–8)

**Goal**: Agent ist nach außen sowohl MCP-Server (Tools) als auch A2A-Peer (Agent-zu-Agent).

**Abhängigkeit**: Phase 4 abgeschlossen.

### Tasks
- [x] **5.1** `bin/agent-mcp`: `rmcp`-Server-Mode mit stdio+HTTP-Transport. → `crates/mcp-server` (`agent-mcp` bin), **stdio** fertig; HTTP-Transport verschoben (mit 5.3 Auth).
- [x] **5.2** Selektive Tool-Exposure (config-driven, default deny). → `~/.ravn/mcp-server.toml` `expose=[…]`; nur `Read`-Tools (Write/Exec gefiltert).
- [ ] **5.3** Auth für MCP-Server (Bearer-Token, IP-Allowlist).
- [x] **5.4** A2A-Endpoint: Agent Card (JSON), `/tasks`/`/messages`/`/artifacts` via JSON-RPC 2.0 über HTTPS. → `crates/a2a` (`a2a-serve`), Agent Card + `message/send` + `message/stream` (SSE) + `tasks/get|cancel`; HTTP (HTTPS via Reverse-Proxy).
- [x] **5.5** A2A-Authentication (OAuth2/OIDC). → optionaler `[auth]`-Block, JWT-Validierung gegen JWKS (issuer/audience/expiry/scopes) via `jsonwebtoken`.
- [ ] **5.6** Multimodal-Input: `MultiModalMessage`-Enum (`Text|Image|Audio`), Image-OCR via Vision-Model.
- [x] **5.7** A2A-Client-Side: Discover + Call externer A2A-Agents. → `A2aClient` (Card-Discovery + `message/send`, OAuth2 client-credentials) + `call_agent`-Tool (von der CLI registriert).

### Akzeptanzkriterien
- Externer MCP-Client (Claude Desktop) findet unsere Tools via stdio.
- A2A-Peer kann via Agent Card eine Task triggern und Result erhalten.
- Image-Upload → OCR-Result → in Conversation-Context.

---

## Phase 6 – RL & Self-Improvement (Monat 9–12)

**Goal**: Trajectory-Logger vollständig; Skill-Synthesis aus Erfolg-Trajektorien; lokales 7B–14B-Modell via GRPO/DAPO finetuned.

**Abhängigkeit**: Phase 5 abgeschlossen; ≥ 5000 reale Trajektorien.

### Tasks
- [ ] **6.1** Trajectory-Logger Schema-Lock: `{trace_id, step, thought, action, observation, reward?}` als JSONL-Export-Pfad zusätzlich zur SQLite.
- [ ] **6.2** Reward-Funktionen für verifizierbare Skills (Tests grün, Git-Commit, File-Diff matched).
- [ ] **6.3** `bin/curator`: nightly Job sucht häufige Action-Sequenzen, abstrahiert zu SKILL.md-Kandidaten.
- [ ] **6.4** Skill-Synthesis-Verification: Dry-Run auf historischen Tasks, Merge nur bei Pass-Rate-Verbesserung.
- [ ] **6.5** Skill-Repo: Git-versioned, atomic Rollback bei Regression.
- [ ] **6.6** Python-Brücke: Trajectory-Export → TRL/Unsloth/verl. PyO3 oder Subprocess.
- [ ] **6.7** GRPO-Pipeline auf lokalem 7B/14B (Qwen2.5/Phi-4-base).
- [ ] **6.8** Rust-Inference des fine-tuned Modells via `mistral.rs` 0.7.x mit ISQ-Quantisierung.
- [ ] **6.9** Constitutional Self-Auditing: User-definierte Verfassung (Markdown) + nightly Auditor-Agent gegen letzte 100 Sessions, Findings → MEMORY.md.

### Akzeptanzkriterien
- Eval-Score Locally-Fine-Tuned vs. Sonnet-Baseline: ≥ 80 % auf einfachen Tasks bei < 5 % Cloud-Kosten.
- Mindestens 1 auto-synthesizer Skill in Production.
- Auditor identifiziert ≥ 1 echten Privacy-/Quality-Issue pro Woche.

---

## Phase 7 – Polish & Release (Monat 13+)

**Goal**: Public-Release-fähig: Tauri-App signed+notarized, E2E-Encryption, optional Speculative Execution, Open-Source-Crates.

**Abhängigkeit**: Phase 6 stabilisiert. Tauri-Basis existiert seit Phase 4.

### Tasks
- [ ] **7.1** Tauri-App Production-Hardening: Code-Signing macOS/Windows, Notarization macOS, Linux-Distribution-Packages (AppImage, .deb, .rpm).
- [ ] **7.2** Auto-Update via Tauri-Updater + Signature-Verification.
- [ ] **7.3** E2E-Encryption: SQLite + Markdown via `rage` (age).
- [ ] **7.4** Cross-Device-Sync via Iroh/IPFS (keys stay local).
- [ ] **7.5** Speculative Tool Execution: idempotente Read-Only-Tools parallel ausführen, Tool-Layer-Cache.
- [ ] **7.6** Open-Source-Release Kandidaten: `agent-graph` (StateGraph + Checkpoint + Visualizer), Trajectory-Logger-Standard.
- [ ] **7.7** Docs, Examples, Contribution-Guide.

### Akzeptanzkriterien
- Tauri-App auf macOS/Linux/Windows signed + notarized, Auto-Update funktioniert.
- E2E: Plaintext nur in Memory, Disk verschlüsselt.
- Mind. 1 Open-Source-Crate veröffentlicht mit eigenständiger CI + Doku.

---

## Querschnitts-Themen (durchgehend ab Phase 0)

- **Cost-Tracking** in jedem LLM-Call (input/output/cache_read/cache_creation/reasoning Tokens).
- **Tracing-Spans** pro Task/LLM-Call/Tool-Call/Subagent.
- **Trajectory-Events** in `events`-Tabelle bei jeder ReAct-Iteration.
- **Anti-Patterns aktiv vermeiden** (siehe project.md §6): Cache-Killer-Timestamps, Tools mit >5 Parametern, Memory unbounded, synchron in async ohne `spawn_blocking`, fehlende Cancellation, MCP-Server-Spam.

---

## Re-Evaluation Triggers (aus project.md §"Benchmarks")

Diese Schwellen lösen eine Strategie-Review aus:

| Trigger | Aktion |
|---|---|
| Cache-Hit-Rate < 60 % | System-Prompt-Stabilität überarbeiten |
| Token-Cost > $0.50/Task | Default-Modell verkleinern, mehr lokal |
| Multi-Agent-Tasks < 5 % | Subagent-Code zurückbauen |
| Skill-Hit-Rate < 30 % | Skills generischer, Routing verbessern |
| Eval-Pass-Drop > 10 % nach Model-Update | Rollback, Re-Kalibrierung |

---

## Notes

- Dieses Dokument ist ein Working-Plan, kein Vertrag. Phasen können sich überlappen; einzelne Tasks dürfen vorgezogen werden, wenn Abhängigkeiten dies erlauben.
- Anti-Pattern aus project.md §6.1 ernstnehmen: „Komplexes Framework wählen, bevor Patterns verstanden." → Phase 0–1 bewusst minimalistisch.
- `rig-core` und `rmcp` sind pre-1.0 — Versionen pinnen, Breaking-Change-Migrations-Notes mitführen.
