//! Pre-step routing (Phase 3.1, D15) + Reflexion-Retry (Phase 3.5).
//!
//! Decides which [`crate::reasoning::Mode`] the agent should use for
//! the next loop iteration. D15 picks pure heuristics — no extra LLM
//! call, fully deterministic, easy to debug. A learned classifier
//! (small LLM trained on trajectories) is a Phase 6 follow-up.
//!
//! Heuristic ([`HeuristicRouter`]):
//! 1. If the most recent tool result was an error AND we still have
//!    reflection budget (default 3 attempts per run) → [`Mode::Reflect`].
//! 2. If the most recent tool result was an error AND reflection cap
//!    is exhausted → [`Mode::Deep`] (escalate, last-ditch effort).
//! 3. Else if the loop has been going for `deep_threshold` steps
//!    without resolving → [`Mode::Deep`].
//! 4. Else → [`Mode::Fast`].

use crate::reasoning::Mode;

/// Default per-run cap on Reflect retries — matches `project.md`
/// "Reflexion (3× Kosten)".
pub const DEFAULT_MAX_REFLECTIONS: usize = 3;

/// Inputs visible to the router at the start of each iteration.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouterInput {
    /// 1-based count of the iteration we're about to start.
    pub step: usize,
    /// `true` iff the previous loop iteration produced at least one
    /// `is_error=true` tool result.
    pub last_iteration_had_tool_error: bool,
    /// `true` if the most recent assistant turn was already produced
    /// in `Mode::Reflect`.
    pub previous_mode_was_reflect: bool,
    /// How many [`Mode::Reflect`] iterations the loop has already run
    /// this turn-chain. Drives the cap before escalating to `Deep`.
    pub reflection_attempts: usize,
}

pub trait Router: Send + Sync {
    fn classify(&self, input: RouterInput) -> Mode;
}

/// Pure heuristic router from D15 + Reflexion-Retry from 3.5.
#[derive(Debug, Clone, Copy)]
pub struct HeuristicRouter {
    /// Steps beyond which we promote to `Mode::Deep`. Default 3.
    pub deep_threshold: usize,
    /// Maximum `Mode::Reflect` retries per run before escalating to
    /// `Mode::Deep`. Default [`DEFAULT_MAX_REFLECTIONS`] = 3.
    pub max_reflections: usize,
}

impl Default for HeuristicRouter {
    fn default() -> Self {
        Self {
            deep_threshold: 3,
            max_reflections: DEFAULT_MAX_REFLECTIONS,
        }
    }
}

impl Router for HeuristicRouter {
    fn classify(&self, input: RouterInput) -> Mode {
        // Failure handling: Reflect up to `max_reflections` times,
        // then escalate to Deep (= the only escape hatch — Reflect
        // didn't help, give the powerful model a try).
        if input.last_iteration_had_tool_error {
            return if input.reflection_attempts >= self.max_reflections {
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
    fn tool_error_triggers_reflect_within_budget() {
        let r = HeuristicRouter::default();
        for attempts in 0..r.max_reflections {
            assert_eq!(
                r.classify(RouterInput {
                    step: 1,
                    last_iteration_had_tool_error: true,
                    previous_mode_was_reflect: attempts > 0,
                    reflection_attempts: attempts,
                }),
                Mode::Reflect,
                "attempt {attempts} should still Reflect"
            );
        }
    }

    #[test]
    fn reflection_budget_exhausted_escalates_to_deep() {
        let r = HeuristicRouter::default();
        assert_eq!(
            r.classify(RouterInput {
                step: 2,
                last_iteration_had_tool_error: true,
                previous_mode_was_reflect: true,
                reflection_attempts: r.max_reflections,
            }),
            Mode::Deep
        );
    }

    #[test]
    fn reflection_cap_is_configurable() {
        let r = HeuristicRouter {
            max_reflections: 1,
            ..HeuristicRouter::default()
        };
        // Attempt 0 of 1 → still Reflect.
        assert_eq!(
            r.classify(RouterInput {
                last_iteration_had_tool_error: true,
                reflection_attempts: 0,
                ..Default::default()
            }),
            Mode::Reflect
        );
        // Attempt 1 of 1 → escalate.
        assert_eq!(
            r.classify(RouterInput {
                last_iteration_had_tool_error: true,
                reflection_attempts: 1,
                ..Default::default()
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
            ..HeuristicRouter::default()
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
