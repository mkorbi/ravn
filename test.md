# Phase 2 Smoketest

Stand: nach Commit `158cffe` (alle Phase-2-Tasks außer 2.3 manuelle
MCP-Verifikation). Wenn alles unten grün ist, ist Phase 2 abgenommen
und wir mergen `v0.2.0 → main`.

---

## Vorbereitung

```bash
export ANTHROPIC_API_KEY=sk-ant-…

# 1) Skills installieren (1× pro Install)
mkdir -p ~/Library/Application\ Support/ravn/skills
cp -R ~/ravn/crates/skills/initial/* ~/Library/Application\ Support/ravn/skills/

# 2) MCP-Server (Filesystem als Test) konfigurieren
cat > ~/Library/Application\ Support/ravn/mcp-servers.toml <<'EOF'
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
env = ["PATH", "HOME"]
permission = "write"
EOF

# 3) ravn bauen und starten
cd ~/ravn
cargo run --release -p ravn-cli
```

Zweites Terminal offen halten für SQL-Verifikation. **Wichtig:**
alle `sqlite3 …` und `cat …` Befehle gehören dorthin, nie in die TUI tippen.

Voller DB-Pfad inline benutzen, keine `$DB`-Variable:
```bash
ls -la ~/Library/Application\ Support/ravn/state.db
sqlite3 ~/Library/Application\ Support/ravn/state.db ".tables"
```

Du solltest **mind. diese Tabellen** sehen: `sessions`, `messages`,
`messages_fts`, `events`, `tool_allowlist`, `skills`, `skills_fts`.

---

## Block A — Startup-Logs

Im **zweiten Terminal**, nachdem ravn gestartet ist:

```bash
tail -30 ~/Library/Application\ Support/ravn/ravn.log
```

Pass-Kriterien:
- [ ] Zeile `INFO  ravn session started session=… model=… db=…` vorhanden
- [ ] Zeile `INFO  skills sync done inserted=3 updated=0 unchanged=0 deleted=0`
  (3 Initial-Skills wurden in die DB gespiegelt)
- [ ] Zeile `INFO  spawning MCP subprocess server=filesystem` vorhanden
- [ ] Zeile `INFO  MCP server connected server=filesystem registered=N`
  (N ist die Tool-Anzahl, typisch 11 für server-filesystem)

Falls "MCP server connect failed" steht: `npx` ist nicht im PATH, oder
`@modelcontextprotocol/server-filesystem` nicht installiert. Quick-Fix:
`npm i -g @modelcontextprotocol/server-filesystem` oder Pfad in der
toml hardcoden.

---

## Block B — Skills sind in der DB

Im **zweiten Terminal**:
```bash
sqlite3 -header -column \
  ~/Library/Application\ Support/ravn/state.db \
  "SELECT id, name, length(body) AS body_bytes, body_hash
     FROM skills ORDER BY name;"
```

Pass-Kriterien:
- [x] 3 Rows: `git-workflow`, `note-taking`, `web-research`
- [x] Jede `body_bytes` > 500 (die SKILL.md-Bodies sind nicht-trivial)
- [x] `body_hash` ist ein 64-stelliger Hex-String

```bash
# Re-Sync-Test: Re-Run ravn (Ctrl-C im 1. Terminal, dann `cargo run` erneut).
# Beim 2. Start sollte ravn.log zeigen:
#   INFO  skills sync done inserted=0 updated=0 unchanged=3 deleted=0
```
- [ ] Re-Run zeigt `unchanged=3` (Body-Hash-Detection funktioniert)

---

## Block C — `skill_list` Tool

In **TUI** tippen:
```
List the skills you have available
```

Pass-Kriterien:
- [x] Dim-Zeile `🔎 skill_list {}` im Scrollback (kein Modal)
- [x] Dim-Zeile `✓ skill_list: 3 skill(s):` folgt
- [x] Assistant zählt `git-workflow`, `note-taking`, `web-research` mit Beschreibung auf

---

## Block D — `skill_view` Tool

In **TUI**:
```
Show me the full git-workflow skill
```

Pass-Kriterien:
- [x] Dim-Zeile `🔎 skill_view {"name":"git-workflow"}`
- [x] Dim-Zeile `✓ skill_view: # git-workflow (skill) …`
- [x] Assistant referenziert den Body inhaltlich (z.B. die "Safety rules"-Section)

---

## Block E — Allowlist persistiert über Session-Restart (2.11, D13)

Erste Session in **TUI**:
```
Write "test1" to /tmp/ravn_allow.txt
```
- [x] Approval-Modal kommt → `a` (allow always)
- [x] Tool läuft, Datei wird geschrieben

Im **zweiten Terminal**:
```bash
sqlite3 ~/Library/Application\ Support/ravn/state.db \
  "SELECT tool_name FROM tool_allowlist;"
```
- [ ] Ausgabe enthält `file_write`

TUI quittieren (Ctrl-C bei leerem Input). Dann **neue Session** starten:
```bash
cargo run --release -p ravn-cli
```

In der neuen TUI:
```
Write "test2" to /tmp/ravn_allow2.txt
```
- [x] **Kein** Approval-Modal — `file_write` läuft direkt durch (Allowlist persistiert)
- [x] `cat /tmp/ravn_allow2.txt` zeigt `test2`

---

## Block F — Auto-Embed + Hybrid Session-Search (2.8, 2.9, 2.10)

Beim ersten Embedding lädt Qwen3-Embedding-0.6B (~1.2 GB) aus HF herunter.
Geduld beim ersten Hit.

In **TUI**:
```
Tell me three interesting facts about marine biology
```
- [x] Antwort streamt durch wie gewohnt

Im **zweiten Terminal**, nach ein paar Sekunden:
```bash
sqlite3 ~/Library/Application\ Support/ravn/state.db \
  "SELECT COUNT(*) FROM messages_vec;"
```

> **Known limitation:** Das `sqlite3` CLI failed mit
> `Error: in prepare, no such module: vec0`, weil sqlite-vec bei uns
> statisch gegen rusqlite gelinkt ist (kein standalone `.dylib`). Der
> Count ist über das CLI nicht abfragbar. Stattdessen direkt über die
> TUI testen (Schritt unten) — wenn `session_search` Hits liefert,
> ist `messages_vec` definitiv gefüllt.

- [X] (Via TUI verifiziert — `session_search` Hits sind der proof)

Jetzt Semantic-Suche testen. In **TUI**:
```
session_search for "ocean creatures"
```
(Das ist ein bewusster reformulierter Begriff, der nicht wörtlich im
ersten Turn vorkommt — pure FTS5 fände das nicht.)
- [X] `🔎 session_search` läuft
- [X] Ergebnis enthält mind. einen Hit aus der "marine biology"-Konversation
  (Beweis: Hybrid + Vec-Index funktioniert)

---

## Block G — MCP-Server tatsächlich verbunden (Task 2.3)

In **TUI**:
```
List the tools you have available
```
- [x] Antwort enthält native Tools (file_read, shell, etc.)
- [x] Antwort enthält MCP-Tools mit Präfix `filesystem__` (z.B. `filesystem__read_file`, `filesystem__write_file`)

---

## Block H — MCP-Tool ausführen

In **TUI**:
```
Use filesystem__list_directory to list /tmp
```
- [x] Approval-Modal kommt (permission=write aus der Config triggert es)
- [x] `y` → Tool läuft, gibt Verzeichnis-Listing zurück
- [ ] Assistant fasst sinnvoll zusammen

```
Use filesystem__read_file to read /tmp/ravn_allow.txt
```
- [ ] (Read-Permission default ist write per Config → erneut Modal)
- [ ] `y` → Inhalt `test1` kommt zurück

---

## Block I — Pro-Server vs Pro-Tool Permission Override

Edit `~/Library/Application Support/ravn/mcp-servers.toml` und füg ein:

```toml
[tools."filesystem__list_directory"]
permission = "read"
```

ravn neustarten. Dann in **TUI**:
```
Use filesystem__list_directory to list /tmp
```
- [ ] **Kein** Modal (Tool-Override greift)

```
Use filesystem__write_file to create /tmp/ravn_mcp.txt with content "hello"
```
- [ ] Modal kommt weiterhin (write bleibt Server-Default)

---

## Block J — Skill-Aktivierung End-to-End

Frisch starten (oder einfach in der bestehenden TUI). Eingabe:
```
I just made some changes to my repo and want to commit them. What's the safe way?
```

Erwartet:
- [X] Assistant ruft `skill_list` mit Filter wie `commit` oder `git` (optional)
- [X] Oder direkt `skill_view {"name":"git-workflow"}`
- [X] Antwort spiegelt die "Safety rules" + "Standard commit flow"-Inhalte aus der SKILL.md (z.B. erwähnt `git status` zuerst, conventional-commit-Format, kein `--no-verify`)

Falls Assistant die Skills nicht von selbst aufruft: explizit anstoßen:
```
Use the skill_list tool first, then read the git-workflow skill
```
Das sollte garantiert klappen.

---

## Block K — Trace-Logging: alles auf einem Blick

Im **zweiten Terminal** nach den ganzen Tests:

```bash
sqlite3 -header -column \
  ~/Library/Application\ Support/ravn/state.db \
  "SELECT kind, COUNT(*) AS n
     FROM events GROUP BY kind ORDER BY n DESC;"
```

Pass-Kriterien:
- [x] `react.tool.start` und `react.tool.end` mit gleicher Anzahl (sofern keine Tool-Crashes)
- [x] `react.done` pro abgeschlossenem Run einmal
- [x] `n` für `react.tool.*` ist > 5 (du hast viele Tool-Calls durchlaufen)

---

## Pass-Fail Phase 2

Phase 2 ist abgenommen wenn:
- A, B, C, D ✓ (Skills round-trip)
- E ✓ (Allowlist-Persistence)
- F ✓ (Embeddings + Hybrid-Search)
- G, H ✓ (MCP-Server + Tool-Calls) — **damit ist Task 2.3 verifiziert**
- I ✓ (Permission-Override)
- J ✓ (LLM nutzt Skills tatsächlich)
- K ✓ (Trajectory-Logging vollständig)

## Falls etwas failed

Pack genaue Error-Message + Block-Letter in die Antwort, ich gucke.
Gängige Fallstricke:
- **MCP `connect failed`**: `npx` nicht im PATH oder server-filesystem nicht
  global installiert. Quick: `npm i -g @modelcontextprotocol/server-filesystem`.
- **Embeddings dauern ewig**: erster Run zieht ~1.2 GB. Logs in
  `ravn.log` zeigen `loading embedding model` mit progress.
- **`no such table: skills`**: Migrations sind nicht gelaufen — DB-File
  löschen und neu starten (verlierst nur Conversation-History).
- **Modal kommt obwohl Allowlist-Eintrag**: TuiApprover hat den Cache
  beim Start nicht geladen — check `ravn.log` auf `failed to preload tool allowlist`.
