---
name: note-taking
description: |
  Use to capture, organize, and recall structured notes — daily logs,
  ideas, project notes, meeting summaries. Persisted so future sessions
  remember.
trigger_patterns: ["take a note", "remember this", "log this", "note that", "save this", "remind me"]
allowed_tools: [memory_save, session_search, file_read, file_write]
---

# Note-Taking

## When to use

The user wants you to remember something across sessions — a fact, a
decision, a TODO, a meeting summary, an idea worth keeping.

## Where notes live

- **Long-term facts and decisions** → `memory_save` with `slot="memory"`.
  These appear in every future session's system prompt.
- **Per-user preferences** (favourite editor, time zone, tone, language)
  → `memory_save` with `slot="user"`.
- **Project notes / topical knowledge** → `file_write` to a Markdown file
  the user names (e.g. `~/notes/<topic>.md`).

## Workflow

1. **Clarify the note's purpose** if ambiguous: is it a one-off fact, a
   recurring preference, or context for a specific project?
2. **Pick the right destination** (see "Where notes live" above).
3. **Format as Markdown** with a clear `## <heading>` line — even
   one-liners. Headings make `session_search` results readable later.
4. **Call the tool** — `memory_save` defaults `section` to today's date;
   pass `section=` explicitly for a topical anchor like
   `section="API design decisions"`.
5. **Confirm** to the user what was saved and where (file path or memory
   slot).

## Recall

When the user later asks "what did I tell you about X":

1. Try `session_search "<keyword>"` — pulls from past conversations.
2. If nothing matches, check `memory.md` / `user.md` directly via
   `file_read` at `~/Library/Application Support/ravn/`
   (macOS) or `$XDG_DATA_HOME/ravn/` (Linux).

## Anti-patterns

- Don't save trivia ("user said hello"). Save things with a non-obvious
  "why".
- Don't append to `memory.md` without a section header — a wall of text
  is unsearchable.
- Don't echo passwords, API tokens, or private keys into `memory_save`.
