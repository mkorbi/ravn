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

DB-Pfad als Shell-Variable (macOS):
```bash
DB="$HOME/Library/Application Support/ravn/state.db"
```

---

## Was beim ersten Durchlauf failed → jetzt retesten

### Block C — Read-Tool ohne Approval (`datetime`)

War failed mit `tool_use ids must be unique`. Sollte jetzt durchlaufen.

In TUI eingeben:
```
What is today's date in Berlin?
```

Pass-Kriterien:
- [ ] Im Scrollback erscheint dim `🔎 datetime {…}` (Tool-Start, **kein** Modal)
- [ ] Dann `  ✓ datetime: 2026-…` (Tool-Ergebnis)
- [ ] Endgültige Assistant-Antwort enthält das aktuelle Datum
- [ ] **Kein** Error-Banner („tool_use ids must be unique" oder ähnlich)
- [ ] **Genau ein** `🔎 datetime`-Block, nicht zwei

---

### Block D — Write-Tool mit Approval (`file_write`)

War tendenziell ok, hatte aber den gleichen 400er.

In TUI eingeben:
```
Write the word "test" to /tmp/ravn_test.txt
```

Pass-Kriterien:
- [ ] Modal erscheint mit `file_write`, `WRITE`, Args pretty-printed
- [ ] `y` → Modal weg, dim Zeile `  ✓ file_write: wrote …`
- [ ] **Genau ein** Modal-Popup, nicht zwei hintereinander
- [ ] Im zweiten Terminal: `cat /tmp/ravn_test.txt` → `test`
- [ ] **Kein** 400-Error

---

### Block H — Allowlist + Single-Invoke

War der eindeutigste Reproducer für den Double-Invoke-Bug.

In TUI eingeben:
```
Run "uname -s" via shell
```
- [ ] Modal → `a` → läuft, dim Zeile `  ✓ shell: exit=0 …`
- [ ] **Genau ein** `⚙ shell {"command":"uname -s"}` Block, nicht zwei

Direkt danach in TUI:
```
Run "whoami" via shell
```
- [ ] **Kein** Modal (Allowlist greift)
- [ ] **Genau ein** `⚙ shell {"command":"whoami"}` Block, nicht zwei
- [ ] **Genau ein** `✓ shell: exit=0` Block

---

### Block J — Trustworthy-Wrap (web_fetch)

Die SQL-Verifikation hat letztes Mal nicht geklappt weil der Command
in die TUI getippt wurde. Diesmal **strikt im zweiten Terminal**.

In **TUI**:
```
Fetch https://example.com and tell me what it says
```
- [ ] Im Scrollback `🔎 web_fetch` (kein Modal, Read)
- [ ] Assistant-Antwort beschreibt den Inhalt

Dann im **zweiten Terminal** (Shell, nicht TUI!):
```bash
sqlite3 "$DB" "SELECT content FROM messages WHERE role='user' ORDER BY id DESC LIMIT 1;"
```
- [ ] Output enthält den String `<tool_result trustworthy="false">` und `</tool_result>` als Wrapper um den web_fetch-Output

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
- [ ] Toolchain: `🔎 web_fetch` → Approval-Modal für `file_write` → `y` → `  ✓ file_write: wrote …`
- [ ] Im zweiten Terminal: `cat /tmp/ravn_title.txt` enthält `Example Domain` oder ähnlich
- [ ] **Kein** 400-Error

---

### Block L — Persistence-Verifikation (im zweiten Terminal)

Vorher unklar formuliert — das hier ist **alles Shell**, nichts davon
in die TUI tippen.

```bash
sqlite3 "$DB" <<'SQL'
.headers on
.mode column
SELECT id, channel, model, input_tokens, output_tokens, cost_usd
  FROM sessions
  ORDER BY started_at DESC
  LIMIT 3;

SELECT COUNT(*) AS msg_count, session_id
  FROM messages
  GROUP BY session_id
  ORDER BY msg_count DESC
  LIMIT 3;

SELECT kind, COUNT(*) AS n
  FROM events
  GROUP BY kind
  ORDER BY n DESC;
SQL
```

Pass-Kriterien:
- [ ] Erste Query: mind. eine Row mit nicht-null `model` und `cost_usd > 0`
- [ ] Zweite Query: mind. ein Wert `msg_count > 2` (User + Assistant + ggf. Tool-Result-User)
- [ ] Dritte Query: die Liste enthält mindestens `react.done`, `react.tool.start`, `react.tool.end` (sofern Block C, D oder K erfolgreich war)

---

### Block M — Cache-Hit-Rate (Klarstellung)

War missverständlich: der Warn-Log soll **NUR** dann auftauchen, wenn
hit_rate < 60 % UND die Conversation mind. 5000 input tokens hatte.
Wenn die Statuszeile bereits `hit ≥ 60%` zeigt, ist Block M
vollständig grün, kein log-File-Check nötig.

Falls hit_rate < 60% (z.B. ganz neue Conversation, kleine Eingaben):
- [ ] Optional: schau in `~/Library/Application Support/ravn/ravn.log`
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
- [ ] In ravn.log steht eine Zeile mit `user.md truncated to 500-token cap`

Aufräumen danach:
```bash
echo "Max prefers German for explanations." > "$HOME/Library/Application Support/ravn/user.md"
```

---

### Block O — Budget-Cap (Klarstellung)

Letztes Mal Frage: „you mean max_tokens?" — nein, **`max_steps`**.
Liegt im Struct `Budget` in `crates/core/src/budget.rs`. Um es zu
testen, in `crates/cli/src/main.rs` nach der Zeile
`let agent_config = AgentConfig::new(model.clone());` temporär einfügen:

```rust
let agent_config = {
    let mut c = agent_config;
    c.budget.max_steps = 2;
    c
};
```

Dann `cargo run --release -p ravn-cli` und in TUI:
```
Run "ls /" then "ls /tmp" then "ls /etc" — each via the shell tool.
```

Pass-Kriterien:
- [ ] Loop bricht ab mit Zeile `error: budget exceeded: max_steps`
  (zweites Tool-Call schlägt noch durch, drittes nicht mehr)

Danach: das Snippet in main.rs wieder rausnehmen / `git checkout
crates/cli/src/main.rs`.

---

## Falls noch was failed

Schreib in `test.md` was passiert ist (Error-Message + welcher Block);
ich gucke nochmal. Wenn alles oben grün, Phase 1 endgültig
abgenommen und wir können Phase 2 (MCP + Skills) starten.
