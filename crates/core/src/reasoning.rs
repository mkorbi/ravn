//! Reasoning mode (Phase 3.2, D15/D16).
//!
//! `Mode` drives the per-iteration model + reasoning-effort choice in
//! the agent loop. The router ([`crate::router`], Phase 3.1) picks a
//! mode before each step; this module just defines the type and how
//! to translate it into a concrete [`ravn_llm::CompletionRequest`].
//!
//! Five modes from `project.md` §1.2:
//! - `Fast` — ReAct on Sonnet/Haiku, no thinking budget. Default.
//! - `Deep` — Anthropic Extended Thinking (Opus + budget) or
//!   OpenAI o-series with high reasoning effort.
//! - `Search` — LATS over ReAct trajectories (Phase 6+).
//! - `Plan` — Plan-and-Execute with sub-tasks (Phase 4+).
//! - `Reflect` — Reflexion-Retry after a failure (Phase 3.5).

use ravn_llm::ReasoningEffort;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Fast,
    Deep,
    Search,
    Plan,
    Reflect,
}

impl Mode {
    /// The model name used by this mode under the default profile
    /// ([D16]). Caller may override via `RAVN_MODEL` / `AgentConfig::model`.
    pub fn default_model(self) -> &'static str {
        match self {
            Mode::Fast | Mode::Plan | Mode::Reflect | Mode::Search => "claude-sonnet-4-6",
            Mode::Deep => "claude-opus-4-7",
        }
    }

    /// Provider-side reasoning effort to attach to this turn. Maps to
    /// `thinking.budget_tokens` for Anthropic, `reasoning_effort` for
    /// OpenAI o-series.
    pub fn reasoning_effort(self) -> Option<ReasoningEffort> {
        match self {
            Mode::Fast | Mode::Plan => None,
            Mode::Deep => Some(ReasoningEffort::High),
            Mode::Reflect => Some(ReasoningEffort::Medium),
            Mode::Search => Some(ReasoningEffort::High),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Fast => "fast",
            Mode::Deep => "deep",
            Mode::Search => "search",
            Mode::Plan => "plan",
            Mode::Reflect => "reflect",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_fast() {
        assert_eq!(Mode::default(), Mode::Fast);
    }

    #[test]
    fn deep_picks_opus_with_high_effort() {
        assert_eq!(Mode::Deep.default_model(), "claude-opus-4-7");
        assert!(matches!(
            Mode::Deep.reasoning_effort(),
            Some(ReasoningEffort::High)
        ));
    }

    #[test]
    fn fast_picks_sonnet_with_no_effort() {
        assert_eq!(Mode::Fast.default_model(), "claude-sonnet-4-6");
        assert!(Mode::Fast.reasoning_effort().is_none());
    }

    #[test]
    fn mode_roundtrips_serde() {
        for m in [Mode::Fast, Mode::Deep, Mode::Search, Mode::Plan, Mode::Reflect] {
            let s = serde_json::to_string(&m).unwrap();
            let back: Mode = serde_json::from_str(&s).unwrap();
            assert_eq!(m, back);
        }
    }
}
