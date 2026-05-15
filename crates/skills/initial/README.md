# Initial Skills

Ship-ready `SKILL.md` definitions for the three Phase 2.7 initial skills.
Each subdirectory mirrors the on-disk layout ravn expects.

## Layout

```
git-workflow/SKILL.md
web-research/SKILL.md
note-taking/SKILL.md
```

## Install

D11 makes the filesystem the canonical source for skills. To enable
these in your ravn install, copy the three directories into your data
dir:

```bash
# macOS
cp -R crates/skills/initial/* ~/Library/Application\ Support/ravn/skills/

# Linux (XDG)
cp -R crates/skills/initial/* "${XDG_DATA_HOME:-$HOME/.local/share}/ravn/skills/"
```

Then start ravn — startup logs will show:

```
INFO  skills sync done  inserted=3 updated=0 unchanged=0 deleted=0
```

After the first run, embeddings populate `skills_vec` fire-and-forget
(requires Qwen3-Embedding-0.6B model — auto-downloaded on first run,
~1.2 GB).

## Customize

Edit any `SKILL.md` in your data dir. The next ravn start picks up
changes via SHA-256 body-hash detection. Skills you remove from disk
get deleted from the DB mirror on next sync.

## Trigger model

When the LLM types into ravn, it sees a `skill_list`-style summary of
all skills in the system prompt (Phase 2.6 progressive disclosure). It
calls `skill_view <name>` to pull in the full body when relevant.
The `trigger_patterns` in the frontmatter are a hint for FTS5 +
semantic ranking, not a hard gate.
