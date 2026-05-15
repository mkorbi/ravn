-- Phase 2.11 / D13: persist the per-session tool allowlist across sessions.
--
-- One row = one tool the user has approved via `a` in the TUI modal.
-- Pure name match for now (no args-pattern); see PLAN.md D13 for the
-- trade-off rationale. Revoke via `/allowlist clear <name>` (Phase 2
-- followup) or direct SQL.

CREATE TABLE tool_allowlist (
    tool_name   TEXT    PRIMARY KEY,
    created_at  INTEGER NOT NULL
);
