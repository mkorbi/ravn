---
title: TUI
description: Keybindings, panes, status line.
sidebar:
  order: 1
---

The ravn TUI is a single-pane chat with an input row and a status
line. Everything is alternate-screen — quitting restores your terminal.

## Layout

```
┌─ ravn — claude-sonnet-4-6 ──────────────────────────────────────────┐
│ <splash block: raven + version + URL + slash-commands>             │
│                                                                    │
│ you: hi                                                            │
│                                                                    │
│ ravn: hello! what can I help with?                                 │
│                                                                    │
│ 🔎 datetime {}                                                     │
│   ✓ datetime: 2026-05-21T11:42:00+02:00                           │
│                                                                    │
│ ravn: today is 21 may 2026.                                       │
└────────────────────────────────────────────────────────────────────┘
┌────────────────────────────────────────────────────────────────────┐
│ > what time is it_                                                 │
└────────────────────────────────────────────────────────────────────┘
 session a1b2c3d4 │ in 1234 out 567 cache_r 890 hit 72% │ $0.0042
```

The status line tracks the current session — token totals come from
the provider's `usage` field and persist into `sessions.cost_usd` so
your spend across sessions is auditable.

## Keybindings

### General

| Key | Action |
|---|---|
| `Enter` | Send the current input |
| `Esc` | Cancel an in-flight LLM stream or close the approval modal (= deny + cancel the run) |
| `Ctrl-C` | While streaming → cancel; while idle → quit |

### Text editing

| Key | Action |
|---|---|
| `←` / `→` | Move cursor one character (UTF-8 aware) |
| `Home` / `End`, `Ctrl-A` / `Ctrl-E` | Jump to start / end of line |
| `Backspace` | Delete the character before the cursor |
| `Delete` | Delete the character under the cursor |
| `Ctrl-U` | Clear the input line |

### Slash-commands

Lines starting with `/` are handled locally — no LLM round-trip.

| Command | Alias | What it does |
|---|---|---|
| `/help` | `/h`, `/?` | List slash-commands |
| `/about` |  | Reprint the startup splash |
| `/clear` | `/cls` | Wipe the scrollback (session keeps running) |
| `/heartbeat` | `/hb` | `list` jobs, `run <name>` now, or `reload` `heartbeats.toml` |
| `/voice` | `/v` | Toggle mic recording → transcript dropped into the input line |
| `/quit` | `/exit`, `/q` | Close ravn |

Slash-commands are case-insensitive.

## The status line in detail

```
session a1b2c3d4 │ in 1234 out 567 cache_r 890 hit 72% │ $0.0042
```

- **session id** — first 8 chars of a UUID. Matches the row in `sessions`.
- **in / out** — raw input and output tokens for the current process,
  summed across every LLM call.
- **cache_r** — input tokens served from Anthropic's prompt cache.
- **hit** — `cache_r / (in + cache_r + cache_creation)`. Under 60 %
  after 5000+ tokens logs a `WARN` in `ravn.log` — usually a sign
  that something dynamic crept into the system prompt and is busting
  the cache.
- **$** — running spend, computed from [`ravn_llm::pricing::cost`].

[`ravn_llm::pricing::cost`]: https://github.com/mkorbi/ravn/blob/main/crates/llm/src/pricing.rs
