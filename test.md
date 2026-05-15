# Phase 1 Retest

Stand: nach Fix-Commit `6caa13e` (deduplicate tool_use blocks). Die
folgenden Punkte waren beim ersten Durchlauf failed oder unklar
beschrieben; mach sie nochmal, der Rest aus PLAN.md kann übersprungen
werden, wenn er beim ersten Mal grün war.

---

## Vorbereitung

Setup in einem Terminal:
```bash
export ANTHROPIC_API_KEY=sk-ant-…
cargo run --release -p ravn-cli
```

Ein **zweites** Terminal offen halten — dort laufen die SQL-Verifikationen.
Das hier ist wichtig: alle `sqlite3 …` und `cat …` Commands gehören
ins zweite Terminal, **nie** in die TUI tippen.

**Pfad-Sanity-Check** (im zweiten Terminal, **bevor** du SQL ausführst):
```bash
ls -la ~/Library/Application\ Support/ravn/state.db
sqlite3 ~/Library/Application\ Support/ravn/state.db ".tables"
```
Du solltest die Datei sehen (≥ 4 KB) **und** die Tabellen-Liste enthält
mindestens `sessions`, `messages`, `messages_fts`, `events`. Wenn nicht,
gibt es ein Pfad-Problem und du hast eine andere DB erwischt — checke
`echo $HOME` und ob `~/Library/Application\ Support/ravn/` existiert.

---

## Was beim ersten Durchlauf failed → jetzt retesten

### Block C — Read-Tool ohne Approval (`datetime`)

War failed mit `tool_use ids must be unique`. Sollte jetzt durchlaufen.

In TUI eingeben:
```
What is today's date in Berlin?
```

Pass-Kriterien:
- [X] Im Scrollback erscheint dim `🔎 datetime {…}` (Tool-Start, **kein** Modal)
- [X] Dann `  ✓ datetime: 2026-…` (Tool-Ergebnis)
- [X] Endgültige Assistant-Antwort enthält das aktuelle Datum
- [X] **Kein** Error-Banner („tool_use ids must be unique" oder ähnlich)
- [X] **Genau ein** `🔎 datetime`-Block, nicht zwei

---

### Block D — Write-Tool mit Approval (`file_write`)

War tendenziell ok, hatte aber den gleichen 400er.

In TUI eingeben:
```
Write the word "test" to /tmp/ravn_test.txt
```

Pass-Kriterien:
- [X] Modal erscheint mit `file_write`, `WRITE`, Args pretty-printed
- [X] `y` → Modal weg, dim Zeile `  ✓ file_write: wrote …`
- [X] **Genau ein** Modal-Popup, nicht zwei hintereinander
- [X] Im zweiten Terminal: `cat /tmp/ravn_test.txt` → `test`
- [X] **Kein** 400-Error

---

### Block H — Allowlist + Single-Invoke

War der eindeutigste Reproducer für den Double-Invoke-Bug.

In TUI eingeben:
```
Run "uname -s" via shell
```
- [X] Modal → `a` → läuft, dim Zeile `  ✓ shell: exit=0 …`
- [X] **Genau ein** `⚙ shell {"command":"uname -s"}` Block, nicht zwei

Direkt danach in TUI:
```
Run "whoami" via shell
```
- [X] **Kein** Modal (Allowlist greift)
- [X] **Genau ein** `⚙ shell {"command":"whoami"}` Block, nicht zwei
- [X] **Genau ein** `✓ shell: exit=0` Block

---

### Block J — Trustworthy-Wrap (web_fetch)

Die SQL-Verifikation hat letztes Mal nicht geklappt weil der Command
in die TUI getippt wurde. Diesmal **strikt im zweiten Terminal**.

In **TUI**:
```
Fetch https://example.com and tell me what it says
```
- [x] Im Scrollback `🔎 web_fetch` (kein Modal, Read)
- [x] Assistant-Antwort beschreibt den Inhalt

Dann im **zweiten Terminal** (Shell, nicht TUI!), Pfad direkt inline:
```bash
sqlite3 ~/Library/Application\ Support/ravn/state.db \
  "SELECT content FROM messages WHERE role='user' ORDER BY id DESC LIMIT 1;"
```

Pass-Kriterien:
- [X] Output enthält den String `<tool_result trustworthy="false">` und `</tool_result>` als Wrapper um den web_fetch-Output

---

### Block K — Multi-Step Task

Beim ersten Mal hat der Assistant zwischendurch file_read aufgerufen
statt direkt file_write. Mit dem Dedup-Fix sollte das stabiler werden,
aber Modellverhalten ist nie 100 % deterministisch. Falls Phase 1
trotzdem failed: reformuliere die Eingabe etwas direkter.

Erster Versuch (gleich wie letztes Mal):
```
Fetch https://example.com and save the page title to /tmp/ravn_title.txt
```

Wenn das wieder failed, expliziter:
```
Use web_fetch to load https://example.com. Then use file_write to write the H1 heading you see to /tmp/ravn_title.txt. Do not read the file afterwards.
```

Pass-Kriterien:
- [X] Toolchain: `🔎 web_fetch` → Approval-Modal für `file_write` → `y` → `  ✓ file_write: wrote …`
- [X] Im zweiten Terminal: `cat /tmp/ravn_title.txt` enthält `Example Domain` oder ähnlich
- [X] **Kein** 400-Error

---

### Block L — Persistence-Verifikation (im zweiten Terminal)

Vorher unklar: `$DB`-Variable war im zweiten Terminal nicht gesetzt
(sie galt nur im ersten). Diesmal hardcoded inline, drei einzelne
Queries statt Heredoc:

```bash
# 1) Sessions
sqlite3 -header -column \
  ~/Library/Application\ Support/ravn/state.db \
  "SELECT id, channel, model, input_tokens, output_tokens, cost_usd
     FROM sessions ORDER BY started_at DESC LIMIT 3;"

# 2) Message counts per session
sqlite3 -header -column \
  ~/Library/Application\ Support/ravn/state.db \
  "SELECT COUNT(*) AS msg_count, session_id
     FROM messages GROUP BY session_id
     ORDER BY msg_count DESC LIMIT 3;"

# 3) Events by kind
sqlite3 -header -column \
  ~/Library/Application\ Support/ravn/state.db \
  "SELECT kind, COUNT(*) AS n
     FROM events GROUP BY kind ORDER BY n DESC;"
```

Pass-Kriterien:
- [X] 1) mind. eine Row mit nicht-null `model` und `cost_usd > 0`
- [X] 2) mind. ein Wert `msg_count > 2` (User + Assistant + ggf. Tool-Result-User)
- [X] 3) Liste enthält mindestens `react.done`, `react.tool.start`, `react.tool.end`

---

### Block M — Cache-Hit-Rate (Klarstellung)

War missverständlich: der Warn-Log soll **NUR** dann auftauchen, wenn
hit_rate < 60 % UND die Conversation mind. 5000 input tokens hatte.
Wenn die Statuszeile bereits `hit ≥ 60%` zeigt, ist Block M
vollständig grün, kein log-File-Check nötig.

Falls hit_rate < 60% (z.B. ganz neue Conversation, kleine Eingaben):
- [x] Optional: schau in `~/Library/Application Support/ravn/ravn.log`
  → erst nach 5000+ input tokens sollte da `WARN … cache hit-rate below 60%` stehen

In jedem anderen Fall ist Block M durch die TUI-Statuszeile schon
verifiziert.

---

### Block N (Hard-Limits-Teil) — Memory-Truncation

War unklar wie groß die Datei sein muss. Hier konkret:

Im zweiten Terminal:
```bash
# 800 chars × 'a' = ~200 tokens. Wir wollen >500 Tokens für user.md (cap 500).
# Also mind. 2200 chars, plus etwas Puffer:
python3 -c "print('Max is a user. ' * 200)" > "$HOME/Library/Application Support/ravn/user.md"
wc -c "$HOME/Library/Application Support/ravn/user.md"
# sollte > 2800 sein
```

Dann TUI neu starten:
```bash
cargo run --release -p ravn-cli
```

Beim Start (vor der ersten Eingabe) im zweiten Terminal:
```bash
tail -20 "$HOME/Library/Application Support/ravn/ravn.log"
```

Pass-Kriterien:
- [x] In ravn.log steht eine Zeile mit `user.md truncated to 500-token cap`

Aufräumen danach:
```bash
echo "Max prefers German for explanations." > "$HOME/Library/Application Support/ravn/user.md"
```

---

### Block O — Budget-Cap

Vorher: `max_steps=2` triggerte nicht, weil das Modell alle drei
shell-Calls in **einen** Assistant-Turn gebatcht hat (Iteration 1:
LLM emittiert 3× tool_use → wir invoken alle 3; Iteration 2: LLM
fasst zusammen → keine tool_uses → ReAct terminiert). Nur 2
Iterationen, also `2 > 2 = false` → kein Abbruch.

Fix: `max_steps = 1`. Dann läuft genau **ein** Tool, beim nächsten
LLM-Call trippt der Cap garantiert.

In `crates/cli/src/main.rs` **Zeile 78**:

```rust
    let agent_config = AgentConfig::new(model.clone());
```

Diese eine Zeile ersetzt du temporär durch **zwei** Zeilen:

```rust
    let mut agent_config = AgentConfig::new(model.clone());
    agent_config.budget.max_steps = 1;
```

(Also: `let` → `let mut`, und eine Zeile drunter `max_steps = 1` setzen.)

Verifikation des Edits im zweiten Terminal:
```bash
grep -n "agent_config\.budget" ~/ravn/crates/cli/src/main.rs
```
Du solltest `agent_config.budget.max_steps = 1;` sehen.

Dann build + run + test:
```bash
cd ~/ravn
cargo run --release -p ravn-cli
```

In der TUI eine einfache Eingabe, die genau **einen** Tool-Call triggert:
```
Run "echo hello" via shell
```

Erwarteter Ablauf:
- Step 1 (= ReAct-Iteration 1): LLM emittiert `tool_use shell` → Approval-Modal → `y` → shell läuft
- Step 2 (= ReAct-Iteration 2): vor dem nächsten LLM-Call trippt der Cap

Pass-Kriterien:
- [ ] Modal kommt für den ersten `shell`-Call → `y`
- [ ] `  ✓ shell: exit=0` Zeile erscheint (Tool ist gelaufen)
- [ ] Direkt danach Loop bricht ab mit `error: budget exceeded: max_steps` im Scrollback
- [ ] Statuszeile zeigt nach Abbruch keine weitere Token-Erhöhung

Aufräumen danach (im zweiten Terminal):
```bash
cd ~/ravn
git checkout crates/cli/src/main.rs
```

---

## Falls noch was failed

Schreib in `test.md` was passiert ist (Error-Message + welcher Block);
ich gucke nochmal. Wenn alles oben grün, Phase 1 endgültig
abgenommen und wir können Phase 2 (MCP + Skills) starten.
