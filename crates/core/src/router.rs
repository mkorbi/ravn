//! Pre-step routing (Phase 3.1, D15).
//!
//! Decides which [`crate::reasoning::Mode`] the agent should use for
//! the next loop iteration. D15 picks pure heuristics — no extra LLM
//! call, fully deterministic, easy to debug. A learned classifier
//! (small LLM trained on trajectories) is a Phase 6 follow-up.
//!
//! Heuristic ([`HeuristicRouter`]):
//! 1. If the most recent tool result was an error → [`Mode::Reflect`].
//! 2. Else if the loop has been going for `deep_threshold` steps
//!    without resolving → [`Mode::Deep`].
//! 3. Else → [`Mode::Fast`].
//!
//! Reflection only triggers once per error — if the model fails twice
//! in a row at the same step, we escalate to `Deep` instead of staying
//! in `Reflect` (avoids loops).

use crate::reasoning::Mode;

/// Inputs visible to the router at the start of each iteration.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouterInput {
    /// 1-based count of the iteration we're about to start.
    pub step: usize,
    /// `true` iff the previous loop iteration produced at least one
    /// `is_error=true` tool result.
    pub last_iteration_had_tool_error: bool,
    /// `true` if the most recent assistant turn was already produced
    /// in `Mode::Reflect` — used to detect that reflection didn't help
    /// so we escalate rather than loop.
    pub previous_mode_was_reflect: bool,
}

pub trait Router: Send + Sync {
    fn classify(&self, input: RouterInput) -> Mode;
}

/// Pure heuristic router from D15.
#[derive(Debug, Clone, Copy)]
pub struct HeuristicRouter {
    /// Steps beyond which we promote to `Mode::Deep`. Default 3.
    pub deep_threshold: usize,
}

impl Default for HeuristicRouter {
    fn default() -> Self {
        Self { deep_threshold: 3 }
    }
}

impl Router for HeuristicRouter {
    fn classify(&self, input: RouterInput) -> Mode {
        // Failure handling first: one Reflect retry, then escalate.
        if input.last_iteration_had_tool_error {
            return if input.previous_mode_was_reflect {
                Mode::Deep
            } else {
                Mode::Reflect
            };
        }
        // Long-running multi-step task → go Deep.
        if input.step >= self.deep_threshold {
            return Mode::Deep;
        }
        Mode::Fast
    }
}

/// Router that always returns the same mode. Used as the default in
/// tests and as a manual-override hook (e.g. user types `/deep` in the
/// TUI in a later phase).
pub struct FixedRouter(pub Mode);

impl Router for FixedRouter {
    fn classify(&self, _input: RouterInput) -> Mode {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_step_is_fast() {
        let r = HeuristicRouter::default();
        assert_eq!(
            r.classify(RouterInput {
                step: 1,
                ..Default::default()
            }),
            Mode::Fast
        );
        assert_eq!(
            r.classify(RouterInput {
                step: 2,
                ..Default::default()
            }),
            Mode::Fast
        );
    }

    #[test]
    fn step_beyond_threshold_is_deep() {
        let r = HeuristicRouter::default();
        assert_eq!(
            r.classify(RouterInput {
                step: 3,
                ..Default::default()
            }),
            Mode::Deep
        );
        assert_eq!(
            r.classify(RouterInput {
                step: 10,
                ..Default::default()
            }),
            Mode::Deep
        );
    }

    #[test]
    fn tool_error_triggers_reflect_once() {
        let r = HeuristicRouter::default();
        assert_eq!(
            r.classify(RouterInput {
                step: 1,
                last_iteration_had_tool_error: true,
                previous_mode_was_reflect: false,
            }),
            Mode::Reflect
        );
    }

    #[test]
    fn repeated_failure_in_reflect_escalates_to_deep() {
        let r = HeuristicRouter::default();
        assert_eq!(
            r.classify(RouterInput {
                step: 2,
                last_iteration_had_tool_error: true,
                previous_mode_was_reflect: true,
            }),
            Mode::Deep
        );
    }

    #[test]
    fn fixed_router_returns_set_mode() {
        let r = FixedRouter(Mode::Plan);
        assert_eq!(r.classify(RouterInput::default()), Mode::Plan);
    }

    #[test]
    fn deep_threshold_is_configurable() {
        let r = HeuristicRouter {
            deep_threshold: 5,
        };
        assert_eq!(
            r.classify(RouterInput {
                step: 3,
                ..Default::default()
            }),
            Mode::Fast
        );
        assert_eq!(
            r.classify(RouterInput {
                step: 5,
                ..Default::default()
            }),
            Mode::Deep
        );
    }
}
