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
- [ ] **1.11** Acceptance-Smoketest end-to-end: Multi-Step-Task laeuft, cancellation funktioniert, cache_read>0 auf zweitem Turn.

### Akzeptanzkriterien
- Multi-Step-Task wie „search web for X, save summary to file" läuft end-to-end (mit `web_fetch` statt `web_search`).
- Cancel-Button in TUI bricht Loop binnen 100 ms ab.
- Approval-Modal erscheint bei `shell`/`file_write`/`memory_save`, nicht bei `file_read`/`web_fetch`/`session_search`/`datetime`.
- Re-Run identischer Conversation zeigt Cache-Hit ≥ 60 %.
- Trajectory-Log: jede ReAct-Iteration als Event mit `(thought, action, observation)` in `events`-Tabelle.

---

## Phase 2 – MCP + Skills (Woche 7–10)

**Goal**: Externe MCP-Server konsumierbar; Skill-Discovery via Progressive Disclosure; semantisches Session-Search.

**Abhängigkeit**: Phase 1 abgeschlossen.

### Tasks
- [ ] **2.1** `mcp::client` über `rmcp` 0.16+ mit stdio+HTTP-Transport.
- [ ] **2.2** MCP-Server-Konfig: `~/.ravn/mcp-servers.toml` mit Befehl, Args, Env-Whitelist.
- [ ] **2.3** Integration-Tests mit drei realen MCP-Servern (Filesystem, GitHub, Playwright).
- [ ] **2.4** `skills`-Crate: `SKILL.md`-Parser (YAML-Frontmatter via `serde_yaml`).
- [ ] **2.5** Skill-Registry mit Top-K Description-Matching (Trie + Embedding-Index).
- [ ] **2.6** Progressive Disclosure: `skill_list` (Metadaten, ~100 Tok/Skill) und `skill_view` (lazy SKILL.md-Load) als Tools.
- [ ] **2.7** 3–5 Initial-Skills: `git-workflow`, `web-research`, `note-taking`, `code-review`, `daily-planning`.
- [ ] **2.8** `fastembed-rs` 5.13+ für Embedding-Generation (BGE-Small default).
- [ ] **2.9** `sqlite-vec` `vec0`-Tabelle für `messages_vec`, Embedding-Pipeline (batch 256).
- [ ] **2.10** Hybrid Session-Search: FTS5 + Vec, RRF-Re-Ranking.
- [ ] **2.11** Approval-Allowlist: User kann Tool+Args-Pattern für künftige Auto-Allow markieren.

### Akzeptanzkriterien
- Agent kann externen MCP-Server (z.B. Playwright) ohne Code-Änderung adden.
- 100 fiktive Skills im Registry → Initial-Prompt-Overhead < 12k Tokens.
- `session_search "topic"` liefert in <100 ms relevante Ergebnisse aus Vorsessions.
- Allowlist persistiert über Sessions in DB.

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
- [ ] **4.7** Voice-In: `whisper-rs` lokal **oder** OpenAI-Whisper-API.
- [ ] **4.8** Voice-Out: `piper-rs` lokal **oder** ElevenLabs HTTP.
- [ ] **4.9** Telegram-Bridge via `teloxide` — Pro-User-Session-Mapping.
- [ ] **4.10** Heartbeat-Scheduler via `tokio-cron-scheduler` — User-definierte Trigger („jeden Morgen 8 Uhr Calendar-Sync").
- [ ] **4.11** Persistent World State: typed Rust-Struct (`Projects`, `OpenTabs`, `WatchTargets`), serialisiert in SQLite.

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
- [ ] **5.1** `bin/agent-mcp`: `rmcp`-Server-Mode mit stdio+HTTP-Transport.
- [ ] **5.2** Selektive Tool-Exposure (config-driven, default deny).
- [ ] **5.3** Auth für MCP-Server (Bearer-Token, IP-Allowlist).
- [ ] **5.4** A2A-Endpoint: Agent Card (JSON), `/tasks`/`/messages`/`/artifacts` via JSON-RPC 2.0 über HTTPS.
- [ ] **5.5** A2A-Authentication (OAuth2/OIDC).
- [ ] **5.6** Multimodal-Input: `MultiModalMessage`-Enum (`Text|Image|Audio`), Image-OCR via Vision-Model.
- [ ] **5.7** A2A-Client-Side: Discover + Call externer A2A-Agents.

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
