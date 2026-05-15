# Architektur- und Implementierungsleitfaden: Ein agentisches KI-System in Rust (2025/2026)

## TL;DR
- **Bauen Sie das System als Cargo-Workspace mit klar getrennten Schichten** (Foundation/Reasoning/Tools/Memory/Skills/Orchestration/Channels/Observability/Safety). Starten Sie mit einem **einfachen, debuggbaren ReAct-Loop** + Prompt-Caching, MCP über `rmcp`, Memory aus Markdown-Dateien (à la OpenClaw) + SQLite-FTS5-Volltextarchiv (à la Hermes), und erweitern Sie schrittweise um Reasoning-Modi, LATS-Suchen für harte Aufgaben, Subagent-Delegation und einen GRPO-Trainings-Loop.
- **Im Rust-Ökosystem** gibt es 2025/26 noch keinen klaren Marktführer auf LangGraph-Niveau – das ist Ihre Innovationschance. Empfohlener Default-Stack: `rig-core` 0.37+ (Agent-Abstraktion) + `rmcp` 0.16+ (MCP) + `tokio`/`axum` + `sqlite-vec` oder `lancedb` 0.23+ (Vektor) + `fastembed-rs` 5.13+ (Embeddings) + `mistral.rs` (lokales Inferenz) + `tracing`/`opentelemetry` + `ratatui` für TUI und `tauri` für Desktop.
- **Wirkliche Innovation gewinnen Sie an drei Stellen**: (1) **Accessibility-Tree-First-Computer-Use** statt Vision (10–100× billiger, robuster), (2) **Speculative Tool Execution** mit Reasoning-Models als Value-Function, (3) **lokale GRPO/DAPO-Trainings-Pipeline** auf den eigenen Trajektorien, die Skills automatisch synthetisiert.

---

## Key Findings

1. **Anthropics „Building Effective Agents" (Schluntz & Zhang, Dez. 2024)** ist weiterhin der konzeptionelle Nordstern: Erst Workflows (Prompt-Chaining, Routing, Parallelisierung, Orchestrator-Workers, Evaluator-Optimizer), erst dann echte Agenten. Zitat: „We recommend finding the simplest solution possible, and only increasing complexity when needed."

2. **Reasoning-Modelle verändern den Loop**: DeepSeek-R1 (arXiv 2501.12948, Jan. 2025) zeigt, dass GRPO auf rein outcome-basierte Rewards den AIME-2024-pass@1 von 15,6 % auf 71,0 % hebt. Konsequenz: **Hybrider Modus** mit schnellem Pfad und tiefem Reasoning-Pfad ist ökonomisch zwingend.

3. **MCP hat sich als Standard durchgesetzt**, A2A (Google, April 2025, seit Juni 2025 Linux Foundation, v1.0 mit AP2-Commerce-Extension) ist die komplementäre Schicht für Agent-zu-Agent-Kommunikation. In Rust ist `rmcp` 0.16+ (offizielles SDK Anthropic + Community) der einzige seriöse Weg.

4. **Memory**: Hermes' 3-Layer-Pattern (frozen Markdown + SQLite-FTS5 + optional Honcho) ist robust. Mem0 (Chhikara et al., arXiv 2504.19413) erreicht laut Paper „26% relative improvements in the LLM-as-a-Judge metric over OpenAI" auf dem LOCOMO-Benchmark; mit Graph-Memory zusätzlich ~2 % höherer Overall-Score. Für Personal Assistant ist **Markdown + SQLite-FTS5 + sqlite-vec** das beste Kosten-Nutzen-Verhältnis.

5. **Skills statt MCP-Server-Spam**: Claude Skills (Okt. 2025) etablieren Progressive Disclosure: Metadaten (~100 Tokens) initial, SKILL.md (<5k) on demand, Ressourcen lazy. 90 %+ Context-Ersparnis im Idle.

6. **Single- vs. Multi-Agent**: Cognition vs. Anthropic (Juni 2025) – Single-Agent reicht für die meisten Tasks; Multi-Agent gewinnt nur bei parallelisierbarer Recherche. Anthropic Engineering schreibt im Blog „How we built our multi-agent research system" (13. Juni 2025): „We found that a multi-agent system with Claude Opus 4 as the lead agent and Claude Sonnet 4 subagents outperformed single-agent Claude Opus 4 by 90.2% on our internal research eval." Der Preis ist klar quantifiziert: „In our data, agents typically use about 4× more tokens than chat interactions, and multi-agent systems use about 15× more tokens than chats."

7. **Rust-Ökosystem 2025/26**: `rig-core` ist mit 6.442 GitHub-Stars (laut 0xPlaygrounds-Profil) führend; `swiftide`, `kalosm`, `mistral.rs`, `fastembed-rs` (864 Stars laut Releases-Page) produktionsreif. **Lücken**: kein dominantes LangGraph-Äquivalent, keine standardisierte Trajectory-Logging-Konvention, kein etabliertes Sandbox-Framework.

---

## Details

### 1.1 Foundation Layer: LLM-Provider-Abstraktion

**Empfohlener Stack**:
- **`rig-core` 0.37+** (0xPlaygrounds): Multi-Provider (OpenAI, Anthropic, Gemini, Ollama, xAI, Groq, DeepSeek, Cohere u. v. m.), Vector-Store-Companions (`rig-qdrant`, `rig-lancedb`, `rig-sqlite`), typed Outputs via `schema_output`/`TypedPrompt`, Streaming Multi-Turn.
- **`async-openai` 0.34+** für 1:1-OpenAI-Parität.
- **`rust-genai` 0.6+** als schlanke Multi-Provider-Alternative.
- **`ollama-rs`** für Ollama lokal.
- **`mistral.rs` 0.7.x** (EricLBuehler) als pure-Rust Inferenz-Engine mit OpenAI-kompatiblem HTTP-Server, MCP-Client integriert, ISQ/UQFF-Quantisierung, NCCL/Ring-Tensor-Parallelism.
- **`candle`** (HuggingFace) für Custom-Modelle.

**Prompt-Caching**: Anthropic erlaubt explizit gesetzte `cache_control: {type: "ephemeral"}`-Marker mit bis zu 4 Breakpoints; 5-Min-TTL default, 1-Std-TTL via Beta-Header `anthropic-beta: extended-cache-ttl-2025-04-11`. Cache-Read-Tokens kosten 10× weniger als fresh Tokens; Anthropic gibt in der Announcement „Prompt caching with Claude" (anthropic.com/news/prompt-caching) an: „reducing costs by up to 90% and latency by up to 85% for long prompts". OpenAI cached automatisch ab 1024 Tokens, 50 % Rabatt. **Cache-stabile Reihenfolge**: Tools → System → Skills-Meta → MEMORY.md → SOUL.md → Static-Wissen → History → User-Turn. Dynamisches (Timestamps, Session-IDs) ans Ende.

**Trait-Skizze**:
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>;
    fn stream(&self, req: CompletionRequest) -> BoxStream<'static, Result<StreamChunk>>;
    fn supports_caching(&self) -> bool;
    fn supports_reasoning(&self) -> bool;
}

pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub cache_breakpoints: Vec<CacheBreakpoint>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_tokens: u32,
    pub temperature: f32,
}
```

**Multi-Provider-Fallback & Cost-Routing**:
- **Tier-1** für komplexe Reasoning (Claude Opus / o3 / R1)
- **Tier-2** für Standard-ReAct (Sonnet / GPT-4o)
- **Tier-3** für Routing/Klassifikation (Haiku / GPT-4o-mini / lokal Phi-4)

### 1.2 Agent Loop / Reasoning Layer

**Vier Single-Agent-Patterns** mit Use-Cases:

| Pattern | Wann | Failure-Mode |
|---|---|---|
| **ReAct** | Open-ended, unbekannte Schritte | Wasted tokens, drift |
| **Plan-and-Execute** | Long-horizon, zerlegbar | Replanning-Overhead |
| **ReWOO** | Vorhersehbare Tool-Sequenz, Latency | Rigide bei Surprises |
| **Reflexion** | Verifizierbare Outcomes, Retry | 3× Kosten, evaluator-bound |
| **LATS** | Coding, Suche, harte Probleme | Sehr teuer, MCTS-Overhead |

**LATS** (Zhou et al., ICML 2024, arXiv 2310.04406) erreicht 92,7 % pass@1 auf HumanEval mit GPT-4 durch MCTS über ReAct-Trajektorien plus LLM als Value-Function plus Reflexion. Als optionaler „deep mode", nicht Default.

**Hybrider Modus**:
```rust
pub enum ReasoningMode {
    Fast,    // ReAct + Sonnet/Haiku
    Deep,    // ReAct + o3/R1, extended thinking
    Search,  // LATS über ReAct-Trajektorien
    Plan,    // Plan-and-Execute mit Subagents
    Reflect, // Reflexion-Retry
}

let mode = router.classify(&task, &context).await?;
let result = match mode {
    ReasoningMode::Fast => fast_loop.run(task).await,
    ReasoningMode::Deep => deep_loop.run(task).await,
    _ => todo!(),
};
```

**Iteration Budgets**: Hard Max-Steps (50 ReAct, 5–10 LATS-Iterationen), Token-Budget, Cancellation via `tokio::sync::CancellationToken` durch gesamten Loop.

**Reasoning-Models integrieren**:
- **OpenAI o-series**: `reasoning_effort: low|medium|high`; kein `temperature`/`top_p`.
- **Anthropic Extended Thinking**: `thinking: {type: "enabled", budget_tokens: 8000}`; Thinking-Blocks **müssen** über Turns erhalten bleiben, sonst Cache-Bruch.
- **DeepSeek R1**: `reasoning_content` separat; bei `<think>`-Tags für Endnutzer filtern.

### 1.3 Tool / Capability Layer

**MCP als zentrales Protokoll** mit `rmcp` 0.16+:

```rust
use rmcp::{tool, ServerHandler, ServiceExt};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchRequest { 
    pub query: String,
    pub limit: Option<u32>,
}

#[tool(tool_box)]
impl WebSearchTool {
    #[tool(description = "Search the web with query")]
    async fn search(
        &self, 
        #[tool(aggr)] req: SearchRequest
    ) -> Result<String, McpError> { /* ... */ }
}
```

**Native vs MCP** – Faustregel:
- **Native** (in-process Rust): Filesystem, Shell, eigene DB, Memory, Skills-Discovery. Performance-/sicherheitskritisch.
- **MCP** (out-of-process): GitHub, Slack, Browser (Playwright MCP), externe APIs. Vorteil: Process-Isolation, Sprachunabhängigkeit.

**Progressive Disclosure** (Claude Skills): 
1. **Level 0**: Skill-Metadaten (~100 Tokens) in System-Prompt.
2. **Level 1**: SKILL.md (<5k) on demand via `skill_view`.
3. **Level 2**: Bundled Ressourcen lazy.

Anthropic: 100 Skills × 5k Tokens = 500k; mit Progressive Disclosure ~10k initial. „With progressive disclosure, having 3 skills costs the same as 1 skill until Claude activates them."

**Sandboxing in Rust**:

| Mechanismus | Latency | Isolation | Rust |
|---|---|---|---|
| **Wasmtime** | µs (cold ms) | Capability-based, memory-safe | `wasmtime`, `wasm-sandbox`, WASI 0.2 |
| **Bubblewrap/Firejail** | ms | Namespaces, seccomp | `tokio::process` + bwrap |
| **Firecracker microVM** | 100ms+ | KVM-Hardware | `firecracker-rs-sdk` |
| **Docker** | s | Containers | `bollard` |

Wasmtime sandboxed by design (kein FS/Net default), WASI Preview 2 erlaubt feine Capabilities.

**Tool-Schema-Generierung** via `schemars`:
```rust
#[derive(Deserialize, JsonSchema)]
pub struct FileReadArgs {
    /// Absolute path to file
    pub path: String,
    /// Max bytes to read
    #[serde(default = "default_limit")]
    pub limit: u64,
}
```

### 1.4 Memory & Context Layer

**Vier-Schichten-Memory**:

| Schicht | Inhalt | Persistenz | Größe |
|---|---|---|---|
| **Working** | Conversation-Buffer | Auto-compact bei N % | ≤ Context |
| **Episodic** | Sessions | SQLite+FTS5, lazy `session_search` | unbounded |
| **Semantic** | Curated (MEMORY.md, USER.md) | Markdown im System-Prompt | 1–3k Tokens |
| **Procedural** | Skills (SKILL.md + scripts) | Filesystem progressive disclosure | unbounded |

**File-First (OpenClaw) vs DB-First (Hermes)**:
- File-First: SOUL.md/MEMORY.md/USER.md plain Markdown, Git-versioniert, User-editierbar. Vorteil: Transparenz, Trust. Nachteil: 1500–3000 Tokens Startup pro Session.
- DB-First: SQLite mit FTS5 über alle Messages, lazy `session_search`. Vorteil: unbounded, schnell. Nachteil: kein Semantic-Matching, kein Entity-Resolution.

**Empfehlung 2025/26**: Hybrid – Markdown für stabile Identity (Cache-freundlich!), SQLite-FTS5 für Sessions, `sqlite-vec` für semantisches Re-Ranking, optional Lightweight-Graph für Entities.

```
~/.myagent/
├── soul.md              # Persona (≤800 Tokens)
├── memory.md            # Long-term facts (≤1500)
├── user.md              # User-Modell (≤500)
├── skills/
│   └── git-ops/
│       ├── SKILL.md     # Frontmatter+Anleitung
│       ├── scripts/
│       └── reference/
└── state.db             # sessions, messages, messages_fts, messages_vec
```

**Vector-DB Optionen in Rust**:

| Crate | Modus | Beste Verwendung |
|---|---|---|
| **`sqlite-vec`** 0.1.x | In-Process SQLite-Extension | Personal Assistants, Single-File |
| **`lancedb`** 0.23+ | Embedded, Arrow+Lance | Mittel-bis-petabyte lokal, Multimodal |
| **`qdrant-client`** 1.16+ | Client-Server gRPC | Mit Qdrant-Daemon |

`sqlite-vec` ist pragmatischster Default: keine Daemons, läuft auf jedem Gerät inkl. WASM/Raspberry Pi.

**Embeddings**: `fastembed-rs` 5.13+ – ONNX-Backend, BGE/E5/Qwen3-Embedding-0.6B/mxbai/MiniLM, Batch (256), DirectML/CUDA/CPU. Für Server: HuggingFace `text-embeddings-inference` (TEI).

**Context-Compression**:
1. **Trigger**: 80 % Context-Limit.
2. **Memory-Flush-Pre-Compress**: separater LLM-Call nur mit Memory-Write-Tools, extrahiert Fakten in MEMORY.md/USER.md *vor* Compression.
3. **Compression**: strukturiert (Tool-Calls + Outcomes + Decisions als Bullets).
4. **Pruning**: alte ToolCall-Outputs zu Hashes komprimieren, Pointer auf SQLite-Original.

### 1.5 Skill / Knowledge Layer

**SKILL.md mit YAML Frontmatter**:
```yaml
---
name: git-workflow
description: |
  Use when the user wants to commit, branch, rebase, manage Git.
trigger_patterns: ["commit", "git status", "merge conflict"]
allowed_tools: [bash, file_read, file_write]
---
# Git Workflow Skill
## When to use
...
## Reference
- scripts/conventional-commit.sh
- reference/branching-strategy.md
```

**Skill-Synthesis aus Trajektorien** (Hermes Curator-Pattern):
1. Trajectory-Logger speichert jeden erfolgreichen Loop (Task → Plan → Tools → Outcome → Feedback).
2. Distillation: nightly Curator-Agent sucht häufige Sequenzen, abstrahiert zu SKILL.md-Kandidaten.
3. Verification: Dry-Run auf historischen Tasks, nur bei Verbesserung gemerged.
4. Versioning: Git-Repo mit atomic Rollback.

**Skill-Registry**: Trie/Vector-Index über Skill-Descriptions, Top-K relevant in Prompt injizieren.

### 1.6 Multi-Agent / Subagent Layer

**Single vs Multi**: Cognitions Walden Yan („Don't Build Multi-Agents", Juni 2025) und Anthropic Engineering („How we built our multi-agent research system", 13. Juni 2025) markieren den State of the Debate. Anthropic quantifiziert klar: „We found that a multi-agent system with Claude Opus 4 as the lead agent and Claude Sonnet 4 subagents outperformed single-agent Claude Opus 4 by 90.2% on our internal research eval" – aber: „In our data, agents typically use about 4× more tokens than chat interactions, and multi-agent systems use about 15× more tokens than chats." Multi-Agent also nur bei read-heavy Parallel-Tasks (Research, Codebase-Exploration), nicht als Default.

**Supervisor + Read-Only-Subagents**:
- Hauptloop hält Conversation-State, plant.
- Subagents: klar abgegrenzte Tasks („Find all callers of `fn foo`"), eigener Context, read-only Tools.
- Geben nur Summary zurück, nicht Rohdaten. **Subagents komprimieren Context, multiplizieren ihn nicht.**

**Verbot von Nested Subagents** (wie Claude Code): nur eine Hierarchie-Ebene.

```rust
pub async fn delegate(
    parent: &Agent,
    task: SubAgentTask,
    tools: Vec<ToolHandle>,
) -> Result<SubAgentResult> {
    let subagent = Agent::builder()
        .system_prompt(task.system_prompt)
        .tools(tools)
        .max_steps(20)
        .read_only(true)
        .build();
    
    let handle = tokio::spawn(async move {
        subagent.run(task.goal).await
    });
    let result = handle.await??;
    Ok(SubAgentResult { summary: result.summary, tokens_used: result.tokens, artifacts: result.artifacts })
}
```

Inter-Agent: `tokio::sync::mpsc`-Channels statt Actor-Frameworks. Backpressure und Cancellation aus tokio direkt.

**A2A-Protokoll** (Google Apr 2025, Linux Foundation Juni 2025, v1.0): Agent Cards (JSON), Tasks, Messages, Artifacts via HTTPS+JSON-RPC 2.0. Für Personal-Assistant optional, wichtig bei Vendor-Agent-Integration.

### 1.7 Orchestration / Workflow Layer

**State Machine vs Event-Driven**:
- **State Machine** (Default, LangGraph-Style): Knoten typed Funktionen, Kanten Bedingungen. Checkpointing/Resume/Visualisierung trivial.
- **Event-Driven**: nur bei asynchronen Multi-Source-Events (Telegram + Voice + Cron).

In Rust **kein etabliertes LangGraph-Äquivalent**. `langchain-ai-rust` 5.0+ hat `langgraph`-Modul, aber wenig idiomatisch. **Open-Source-Lücke und Beitrag-Chance.**

```rust
pub struct StateGraph<S: AgentState> {
    nodes: HashMap<NodeId, Box<dyn Node<S>>>,
    edges: Vec<(NodeId, Box<dyn Fn(&S) -> NodeId + Send + Sync>)>,
    entry: NodeId,
}

#[async_trait]
pub trait Node<S>: Send + Sync {
    async fn run(&self, state: &mut S, ctx: &Context) -> Result<()>;
}

impl<S: AgentState> StateGraph<S> {
    pub async fn run(&self, mut state: S) -> Result<S> {
        let mut current = self.entry;
        while current != NodeId::END {
            self.nodes[&current].run(&mut state, &self.ctx).await?;
            self.checkpoint(&state).await?;
            current = self.dispatch(&state)?;
        }
        Ok(state)
    }
}
```

**Checkpointing**: pro Knoten-Übergang Serialize-Snapshot (postcard/msgpack) in SQLite. Crash → deserialisiere → resume.

### 1.8 Persistence Layer

- **SQLite** via `rusqlite` (sync, simpel) oder **`sqlx`** (async, compile-time-checked). Für Desktop fast immer SQLite – Single-File, FTS5 inkludiert.
- **WAL-Mode**: `PRAGMA journal_mode=WAL` für konkurrente Reader.
- **Append-only Audit Log**: separate `events`-Tabelle, niemals UPDATE/DELETE.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY, started_at INTEGER, ended_at INTEGER,
    channel TEXT, model TEXT,
    input_tokens INTEGER, output_tokens INTEGER, cost_usd REAL
);
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT REFERENCES sessions(id),
    role TEXT, content TEXT, tool_calls TEXT,
    reasoning_tokens INTEGER, created_at INTEGER
);
CREATE VIRTUAL TABLE messages_fts USING fts5(content, content='messages', content_rowid='id');
CREATE VIRTUAL TABLE messages_vec USING vec0(embedding float[768]);
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trace_id TEXT, kind TEXT, payload BLOB, created_at INTEGER
);
```

Für Trajectory-Replay: jede ReAct-Iteration als Event mit `(thought, action, observation)`-Tripel.

### 1.9 Interface / Channel Layer

- **TUI**: `ratatui` 0.28+ – Standard. Streaming via `tokio::sync::mpsc`.
- **Desktop GUI**: `tauri` 2.0 (Web-Tech), `dioxus` 0.6+ (Rust-React), `egui` (Immediate-Mode), `leptos` (SSR+Hydration).
- **WebSocket**: `axum` + `tokio-tungstenite`. Pro Verbindung ein Task + mpsc-Channel zur Engine.
- **Messenger**: `teloxide` (Telegram), `serenity`/`twilight` (Discord), `slack-morphism` (Slack).
- **Voice**: STT via `whisper-rs` (FFI) oder `candle-whisper`; TTS via ElevenLabs HTTP oder `piper-rs`.
- **Browser**: `chromiumoxide` 0.7 (CDP-direkt, schnell), `thirtyfour` 0.32-rc (WebDriver Multi-Browser).
- **Desktop-Automation**:
  - Input: `enigo` 0.6+ (Linux X11/Wayland-exp, macOS, Windows).
  - Screen: `xcap` (Nachfolger `screenshots`-Crate, Linux/macOS/Windows).
  - Accessibility: `uiautomation` (Windows), `atspi` 0.28+ (Linux), für macOS direkt `objc2` auf AXUIElement.
- **MCP-Server-Mode**: `rmcp` mit `server`-Feature, stdio/HTTP-Transport.

### 1.10 Observability Layer

- **Tracing**: `tracing` + `tracing-subscriber` + `opentelemetry-otlp` für Jaeger/Tempo-Export.
- **Spans**: pro Task, LLM-Call, Tool-Call, Subagent.
- **Token/Cost-Tracking**: Response-Usage extrahieren (`input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`, `reasoning_tokens`). In `sessions.cost_usd` aggregieren mit Pricing-Tabelle.
- **Trajectory Logging** (für RL!): jede Iteration als `{trace_id, step, thought, action, observation, reward?}`-Event in `events`-Tabelle (postcard binär) + JSONL-Export.
- **Eval/Benchmark**: eigene Eval-Sets (häufige Tasks aufzeichnen, Ground-Truth annotieren); LLM-as-Judge mit Rubriken („did the agent identify all 3 files?"); nightly Cron.

### 1.11 Safety / Approval Layer

**Drei-Stufen-Permission**:
1. **Read** (file_read, search, fetch) – kein Approval.
2. **Write** (file_write, db_update, send_message) – Approval oder Allowlist.
3. **Exec** (shell, browser_navigate, desktop_click) – immer Approval außer Allowlist.

```rust
pub enum Permission { Read, Write, Exec }
pub trait Tool {
    fn permission(&self) -> Permission;
    async fn approve(&self, args: &Value, ctx: &Context) -> Result<bool> {
        match self.permission() {
            Permission::Read => Ok(true),
            Permission::Write => ctx.user_approval_or_allowlist(self, args).await,
            Permission::Exec => ctx.user_approval(self, args).await,
        }
    }
}
```

**Prompt-Injection-Detection**:
- Heuristik: Tool-Outputs scannen auf „ignore previous instructions"-Patterns.
- Strukturell: in `<tool_result trustworthy="false">` wrappen; System-Prompt erklärt untrusted-Behandlung.
- Optional: LLM-as-Judge auf Tool-Outputs.

**Output-Filter**: PII-Scrubbing (Regex IBAN/Mail) vor Output an Drittparteien.

---

## 2. Innovations-Möglichkeiten

### 2.1 Rust-natives Agent-Framework (LangGraph-Niveau)
Keine dominante Lösung. `rig` deckt LLM+Tools+RAG, aber kein typed `StateGraph` mit Checkpointing/Visualization. **Konkrete Lücke**: Open-Source-Crate `agent-graph` mit typed Knoten, Checkpoint-Trait, Time-Travel-Debug, ratatui-Visualizer.

### 2.2 Verifiable-Reward-RL auf lokalen Trajektorien (GRPO/DAPO)
DeepSeek-R1 zeigte: GRPO + rule-based outcome rewards reicht für SOTA-Reasoning. Auf Ihren Trajektorien können Sie:
- Verifizierbare Skills definieren (`git_commit` → success = commit + tests grün).
- Trajektorien aus echter Nutzung sammeln (Wochen/Monate).
- Lokales 7B–14B-Modell mit GRPO finetunen.
- Pipeline: Rust-Inferenz (`candle`/`mistral.rs`) + Python-Brücke zu TRL/Unsloth/verl.

**Die unbesetzte Innovation 2026.**

### 2.3 Differential Reasoning
Reasoning-Models ~10× Kosten. Vor jedem Schritt klassifizieren ob „deep thinking" nötig:
- Schritt-Tiefe < 3 → fast
- Tool-Output ambig → deep
- User-Feedback negativ → deep

Forschungsfeld: kleiner Klassifikator (300M) trainiert auf Trajektorien.

### 2.4 Token-Effiziente Memory via Semantisches Diff
Statt vollen Updates: JSON-Patch (`{op: add, path: /preferences/languages, value: rust}`). Spart 70 %+.

### 2.5 Live Skill Synthesis
Runtime-Generierung neuer Skills nach erfolgreichen Trajektorien: Trajectory → Curator → Draft → Dry-Run → atomic merge.

### 2.6 Multi-Modal Native
Built-in Pipelines mit Whisper (STT), Llama-Vision via `mistral.rs`, Piper (TTS). `MultiModalMessage`-Trait mit `Text | Image | Audio`.

### 2.7 Computer-Use ohne Vision (Accessibility-Tree-First)
**Höchste praktische ROI.** Vision-basiert kostet 10–20k Tokens/Screenshot, fragil. Microsoft Playwright MCP, ChatGPT Atlas, Perplexity Comet nutzen Accessibility-Tree primär.

Stack: `uiautomation` (Windows), `atspi` (Linux), `objc2`→AXUIElement (macOS), Vision als Fallback.

### 2.8 Lokal-First E2E-Encryption
SQLite + Markdown via `age` (Rust: `rage`). Cross-Device-Sync via Iroh/IPFS. Schlüssel nie zu Cloud.

### 2.9 Sub-LLM Capability Routing
Kleine Modelle (Phi-4, Llama-3.2-3B) als Router: 1ms statt 500ms, klassifizieren Tools+Skills. Erst dann großes Modell. 60 %+ Kostenersparnis.

### 2.10 Speculative Tool Execution
Bei mehreren möglichen Aktionen parallel ausführen, bevor LLM final entscheidet. Funktioniert für **idempotente Read-Only-Tools**. Caching auf Tool-Layer nötig, sonst Kostenexplosion.

### 2.11 A2A-Protokoll-Support
Google A2A v1.0 Endpoint. Macht Ihren Agent zum Peer für fremde Agents (Salesforce, MS Copilot, ServiceNow). Rust-Impl noch nicht existent.

### 2.12 Persistent World State
Typed Rust-Struct, nicht nur Conversation-History. Aktuelle Projekte, offene Tabs, Watch-Targets. Heartbeats (OpenClaw) lesen State und reagieren proaktiv.

### 2.13 Constitutional Self-Auditing
Nightly Auditor-Agent gegen User-definierte Verfassung (Markdown) auf letzte 100 Sessions. Findings als MEMORY.md-Updates oder Skill-Disabling.

### 2.14 Inference-Time Search (LATS)
ReAct-Trajektorien als Tree, LLM als Value-Function, MCTS für vielversprechendste Pfade. Funktioniert für verifizierbare Tasks (Tests pass).

---

## 3. Rust-Crate-Empfehlungen (Stand Q2 2026)

| Bereich | Crate | Version | Warum |
|---|---|---|---|
| LLM-Multi-Provider | `rig-core` | 0.37+ | Führend (6.442 Stars laut 0xPlaygrounds GitHub-Profil), 20+ Provider |
| OpenAI 1:1 | `async-openai` | 0.34+ | Vollständige OpenAPI-Spec |
| Multi-Provider light | `genai` | 0.6+ | Schlank, Reasoning-Effort |
| Lokale Inferenz | `mistral.rs` | 0.7.x | Pure Rust, OpenAI-API, MCP-Client |
| ML-Framework | `candle` | aktuell | Custom-Modelle |
| Ollama | `ollama-rs` | latest | Lokal |
| MCP | `rmcp` | 0.16+ | Offizielles SDK (modelcontextprotocol/rust-sdk) |
| Async | `tokio` | 1.x | Standard |
| Async-Traits | `async-trait` | 0.1+ | Default |
| HTTP-Client | `reqwest` | 0.12+ | Default |
| HTTP-Server | `axum` | 0.7+ | Tower-basiert |
| DB sync | `rusqlite` | 0.32+ | Simple SQLite |
| DB async | `sqlx` | 0.8+ | Compile-time-checked |
| Vector embedded | `sqlite-vec` | 0.1+ | Kleine Apps |
| Vector embedded | `lancedb` | 0.23+ | Mittel-bis-groß lokal |
| Vector C/S | `qdrant-client` | 1.16+ | Mit Daemon |
| Embeddings lokal | `fastembed-rs` | 5.13+ | ONNX, BGE/E5/Qwen3 (864 Stars laut Releases-Page) |
| Serde | `serde`/`serde_json` | latest | Standard |
| JSON Schema | `schemars` | 0.8+ | Tool-Defs |
| Tracing | `tracing`/`tracing-subscriber` | latest | Standard |
| OpenTelemetry | `opentelemetry-otlp` | latest | Export |
| TUI | `ratatui` | 0.28+ | De-facto |
| Desktop | `tauri` | 2.0+ | Web-Frontend |
| Desktop Rust | `dioxus` | 0.6+ | React-like |
| Browser CDP | `chromiumoxide` | 0.7+ | Direktes CDP |
| Browser WebDriver | `thirtyfour` | 0.32-rc | Multi-Browser |
| Input-Sim | `enigo` | 0.6+ | Cross-Platform |
| Screen-Capture | `xcap` | latest | Cross-Platform |
| Windows UIA | `uiautomation` | latest | Accessibility |
| Linux AT-SPI | `atspi` | 0.28+ | Accessibility |
| Sandboxing | `wasmtime` | 25+ | WASM-Plugins |
| Sandboxing high | `wasm-sandbox` | latest | Wrapper |
| Markdown | `pulldown-cmark`/`comrak` | latest | GFM |
| Cron | `tokio-cron-scheduler` | latest | Heartbeats |
| YAML | `serde_yaml` | latest | SKILL.md Frontmatter |
| Indexing/RAG | `swiftide` | 0.27+ | Streaming Pipelines |
| Lokal-Toolkit | `kalosm` | 0.4+ | Vision/Audio/Text |
| Telegram | `teloxide` | latest | Mature |
| Whisper | `whisper-rs` | latest | STT lokal |
| Encryption | `rage` (age) | latest | E2E |

---

## 4. Cargo-Workspace-Skizze

```
my-agent/
├── Cargo.toml                  # workspace
├── crates/
│   ├── core/                   # Agent-Loop, Reasoning
│   │   └── src/
│   │       ├── loop.rs         # ReAct/Plan-and-Execute/LATS
│   │       ├── state.rs        # AgentState trait
│   │       ├── budget.rs       # Token/Step/Cost-Limits
│   │       └── router.rs       # Fast vs Deep classifier
│   ├── llm/                    # Provider-Abstraktion
│   │   └── src/
│   │       ├── provider.rs     # LlmProvider trait
│   │       ├── openai.rs / anthropic.rs / ollama.rs / mistralrs.rs
│   │       ├── caching.rs      # Prompt-Cache-Helper
│   │       └── retry.rs
│   ├── tools/                  # Native Tools
│   │   └── src/{fs,shell,db,http,browser}.rs
│   ├── mcp/                    # MCP Client+Server
│   │   └── src/{client,server,registry}.rs
│   ├── memory/                 # Memory-Schichten
│   │   └── src/{working,episodic,semantic,procedural}.rs
│   ├── skills/                 # Skill-System
│   │   └── src/{definition,registry,runtime,synthesis}.rs
│   ├── persistence/            # DB-Layer
│   │   └── src/{schema,repo,checkpoint}.rs
│   ├── orchestration/          # State-Machine
│   │   └── src/{graph,node,subagent}.rs
│   ├── gateway/                # WebSocket+HTTP
│   │   └── src/{ws,http,auth}.rs
│   ├── cli/                    # TUI (ratatui)
│   ├── desktop/                # Tauri-App (optional)
│   ├── computer_use/           # Accessibility+Browser+Input
│   │   └── src/{browser,desktop,a11y}.rs
│   ├── safety/                 # Approval/Permissions
│   │   └── src/{approval,permissions,injection,sandbox}.rs
│   ├── observability/          # tracing+metrics+eval
│   │   └── src/{tracing,cost,trajectory,eval}.rs
│   └── bin/
│       ├── agent-cli/          # TUI-Binary
│       ├── agent-server/       # Gateway-Binary
│       └── agent-mcp/          # MCP-Server-Binary
└── README.md
```

```toml
[workspace]
members = ["crates/*", "crates/bin/*"]
resolver = "2"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "0.8"
tracing = "0.1"
sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio-rustls"] }
rmcp = { version = "0.16", features = ["server","client","macros","schemars"] }
rig-core = "0.37"
```

---

## 5. Inkrementelle Roadmap

### Phase 0 – Foundation (Wo. 1–2)
- Cargo-Workspace, 4 Kern-Crates (core/llm/persistence/cli).
- `LlmProvider`-Trait + OpenAI + Anthropic.
- SQLite-Schema (sessions, messages, events, FTS5).
- ratatui-TUI mit Streaming.
- Tracing + Cost-Tracking ab Tag 1.

### Phase 1 – MVP (Wo. 3–6)
- Basic ReAct-Loop mit Step-Limit/Token-Budget.
- 5–8 native Tools: file_read/write, shell (Approval), web_search, web_fetch, session_search, memory_save, datetime.
- SOUL.md/MEMORY.md/USER.md-Loader.
- Anthropic-Prompt-Caching korrekt.
- Cancellation via `CancellationToken`.

### Phase 2 – MCP + Skills (Wo. 7–10)
- `rmcp` Client: konsumiert externe MCP-Server (Filesystem-MCP, GitHub-MCP, Playwright-MCP).
- Skills mit Progressive Disclosure (`skill_list`, `skill_view`).
- SQLite-FTS5 + `sqlite-vec` für Session-Search.
- Approval-System mit Allowlist.

### Phase 3 – Subagents + Reasoning (Monat 3–4)
- Subagent-Delegation (read-only, isolierter Context).
- Hybrid-Modus: Fast (Sonnet/Haiku) vs Deep (o3/R1/Opus Extended Thinking).
- Reflexion-Retry bei Failure.
- StateGraph + Checkpointing.

### Phase 4 – Computer Use + Multi-Channel (Monat 5–6)
- Accessibility-Tree-First Computer Use (Linux+Windows).
- Browser via `chromiumoxide`.
- Voice (Whisper-STT + Piper-TTS) + Telegram-Bridge.
- Heartbeat-Scheduler für proaktive Aktionen.

### Phase 5 – Eigener MCP-Server + A2A (Monat 7–8)
- `rmcp`-Server-Mode: eigene Tools nach außen.
- A2A-Endpoint (Agent Card, Task/Message).
- Multimodal: Bilder als Input, OCR.

### Phase 6 – RL & Self-Improvement (Monat 9–12)
- Trajectory-Logger vollständig.
- Curator-Agent für Skill-Synthesis.
- GRPO/DAPO-Pipeline auf lokalem 7B–14B (Rust-Inferenz, Python-Training via TRL/verl).
- Constitutional Self-Auditing nightly.

### Phase 7 – Polish (Monat 13+)
- Tauri-Desktop-App.
- E2E-Encryption + Cross-Device-Sync.
- Speculative Tool Execution.
- Open-Source-Release.

---

## 6. Häufige Fallstricke und Anti-Patterns

1. **Komplexes Framework wählen, bevor Patterns verstanden.** Anthropic: „start by using LLM APIs directly". ReAct nackt zuerst.
2. **Multi-Agent als Default.** ~15× Tokens (Anthropic-Daten) für oft marginalen Nutzen.
3. **Cache-Killer**: dynamische Timestamps/IDs im System-Prompt zerstören Anthropic-Cache.
4. **Tools mit zu vielen Parametern oder unklarer Beschreibung.** Max 5 Parameter, Description = ein Satz + Beispiel.
5. **Memory unbounded.** OpenClaw: 5000–8000 Tokens Startup. Hard Limit setzen (Hermes: 1300).
6. **Subagents spawnen Subagents.** Verboten – exponential blow-up.
7. **Vision-First Computer Use.** 10–20k Tokens/Screenshot. A11y-Tree first.
8. **Reasoning-Models immer.** o3/R1 für jeden Schritt = 10× Kosten.
9. **Keine Trajectory-Logs.** RL später unmöglich.
10. **Skills als Mega-Markdown.** Max ~500 Zeilen SKILL.md, Rest in reference/scripts/.
11. **Prompt-Injection-Naivität.** Tool-Outputs in `<tool_result trustworthy="false">`-Wrapper.
12. **Synchron in async.** `rusqlite` in tokio-Task ohne `spawn_blocking` blockt Executor.
13. **Keine Cancellation.** User klickt Cancel, Agent läuft 5min weiter.
14. **MCP-Server-Spam.** Native für High-Frequency, MCP für High-Variety.

---

## 7. Stand des Rust-AI-Ökosystems 2025/26

**Reif**:
- LLM-Clients: `rig-core`, `async-openai`, `genai` produktionsreif.
- MCP: `rmcp` funktional vollständig (pre-1.0).
- Inferenz: `mistral.rs`, `llama-cpp-rs`-Familie.
- Embeddings: `fastembed-rs`, `text-embeddings-inference`.
- Vector: `sqlite-vec`, `lancedb`, `qdrant-client`.
- UI: `ratatui`, `tauri 2.0`.
- ML-Framework: `candle`, `burn`.

**Lücken (Innovationschancen)**:
- Kein dominantes LangGraph-Äquivalent.
- Keine standardisierte Trajectory-Logging-Convention.
- Kein Rust-natives Eval-Framework (a la `inspect_ai`).
- macOS-Accessibility-Tree-Crate fehlt.
- A2A-Protokoll-Impl noch nicht existent.
- Kein offener RL-Loop für Agent-Trajektorien.
- Speculative Tool Execution als Library/Pattern.

**Vorbild-Projekte**:
- **Zed Editor** (Rust-IDE mit AI) – Cloud-Backend in Rust, exzellente UI-Perf.
- **Helix Editor** – modaler Editor, Architektur-Vorbild.
- **mistral.rs** – Pure-Rust Inferenz, OpenAI-API-kompatibel.
- **Probe** – 100 % Rust lokale Code-Search via rig.
- **VT Code** – Rust-Terminal-Coding-Agent mit Tree-sitter + ast-grep + rig.
- **Listen** – Rust-Trading-Agent-Framework.
- **Tantivy** – Rust-Volltext-Search, Library-API-Vorbild.
- **burn** – ML-Framework, Backend-Abstraktion-Vorbild.

---

## Recommendations

### Kurzfristig (2 Wochen)
1. Cargo-Workspace aufsetzen mit 4 Kern-Crates `core`/`llm`/`persistence`/`cli`.
2. ReAct-Loop **nackt** ohne Framework implementieren (direkt aus `async-openai`/`rig-core`).
3. Tracing + Trajectory-Logging ab Tag 1: `tracing` + SQLite `events`.
4. Prompt-Cache-Marker korrekt setzen (Anthropic `cache_control` am Ende von Tools+System-Prompt). Anthropic dokumentiert in „Prompt caching with Claude": „reducing costs by up to 90% and latency by up to 85% for long prompts" – das Caching ist das mit Abstand höchste Cost-Hebel des Stacks.

### Mittelfristig (1–3 Monate)
5. MCP-Integration via `rmcp` – zuerst konsumieren (Filesystem, GitHub, Playwright), bevor eigene bauen.
6. Skills-System mit Progressive Disclosure (3–5 Initial-Skills).
7. Memory-Hybrid (Markdown + SQLite-FTS5 + sqlite-vec). Auto-Compact bei 80 %.
8. Approval-System mit Allowlist + User-Confirm.

### Langfristig (3–12 Monate)
9. Subagents für isolierte Read-Heavy-Tasks. Anthropic schätzt im Engineering-Blog vom 13. Juni 2025 den Cost-Multiplier konkret auf „about 15× more tokens than chats" – also nur einsetzen, wenn Quality-Gain das rechtfertigt.
10. Hybrid Reasoning Mode: Router-Klassifikator entscheidet Fast vs Deep.
11. Accessibility-Tree-First Computer-Use – höchster ROI.
12. Trajectory-basierte Skill-Synthesis (Curator-Agent).

### Visionär (12+ Monate)
13. GRPO-Fine-Tuning eines lokalen 7–14B-Modells auf eigenen Trajektorien.
14. Open-Source-Release der Kern-Crates (StateGraph + Trajectory-Logger).

### Benchmarks/Thresholds, die Entscheidungen ändern
- **Cache-Hit-Rate < 60 %** → System-Prompt zu volatil, Strategie überarbeiten.
- **Token-Cost > $0.50/Task** → kleinerer Default, mehr lokal.
- **Multi-Agent-Tasks < 5 %** → Subagent-Code zurückbauen.
- **Skill-Hit-Rate < 30 %** → Skills zu spezifisch oder Routing schlecht.
- **Eval-Pass-Drop > 10 %** nach Model-Update → Rollback, neu kalibrieren.

---

## Caveats

- **Rust-AI-Ökosystem ist 2025 in Bewegung**: `rig` 0.31→0.37 brachte Breaking Changes; `rmcp` pre-1.0 (Migration-Guide für 1.x existiert bereits); `lancedb` ≥0.20 hat MSRV-Anforderungen (Rust ≥1.91 transitiv durch aws-smithy). Versionen pinnen.
- **Prompt-Caching ist provider-spezifisch und ändert sich**: 1-Std-TTL ist auf Bedrock zum Stand der Recherche nicht verfügbar; Beta-Header können sich ändern. Im Provider-Adapter kapseln.
- **Reasoning-Models verschwenden Tokens falsch eingesetzt.** Klassifikator-Vorschaltung Pflicht.
- **Subagent-Token-Inflation real**: Anthropics eigene Daten: „multi-agent systems use about 15× more tokens than chats". Multi-Agent nicht als Default.
- **MCP-Server-Qualität variiert.** Third-Party-MCP-Server sind Prompt-Injection-Vektoren. Permissions konsequent, Outputs untrusted.
- **GRPO/RL ist Forschungsstand**: DeepSeek-R1-Erfolg auf 671B-Modell; kleinere Modelle (1–10B, vgl. arXiv 2503.16219 „Reinforcement Learning for Reasoning in Small LLMs") zeigen nicht immer dieselbe Self-Evolution. Lang-Term-Bet.
- **macOS Accessibility-Tree in Rust** unausgereift. Möglicherweise Swift-Bridge nötig.
- **Lokale Inferenz für Reasoning** 2025 limitiert: 7B-Modelle oft unzureichend. Hybrid Cloud (Reasoning) + Lokal (Routing/Embeddings) pragmatisch.
- **`sqlite-vec` pre-v1** mit angekündigten Breaking Changes. Trotzdem heute einsetzbar, Migration-Pfad einplanen.
- **Browser-Automation gegen Bot-Detection**: Stealth-Forks ändern sich häufig; rechtliche Klarheit prüfen.
- **A2A vs MCP**: komplementär, keine Konkurrenten. MCP für Tool-zu-Agent, A2A für Agent-zu-Agent.
- **Mem0-Benchmark spezifisch**: die zitierten „26% relative improvements in the LLM-as-a-Judge metric over OpenAI" (arXiv 2504.19413) beziehen sich auf den LOCOMO-Benchmark; auf anderen Memory-Benchmarks können Ergebnisse abweichen. Vor Adoption auf eigenen Workloads validieren.