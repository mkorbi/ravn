//! Step- / Token- / Cost-Budgets fuer den ReAct-Loop (Phase 1.2).
//!
//! Budgets sind harte Caps — werden sie ueberschritten, terminiert der
//! Loop sofort und emittiert [`crate::event::LoopEvent::BudgetExceeded`].
//! Defaults stammen aus `project.md` §1.2 (Hard Max-Steps 50) und sind
//! konservativ; Caller koennen sie via [`Budget`] aendern.

use ravn_llm::Usage;

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_steps: usize,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_cost_usd: f64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: 50,
            max_input_tokens: 200_000,
            max_output_tokens: 50_000,
            max_cost_usd: 1.00,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetUsage {
    pub steps: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
}

pub struct BudgetTracker {
    pub limits: Budget,
    pub usage: BudgetUsage,
}

impl BudgetTracker {
    pub fn new(limits: Budget) -> Self {
        Self {
            limits,
            usage: BudgetUsage::default(),
        }
    }

    pub fn bump_step(&mut self) -> Result<(), &'static str> {
        self.usage.steps += 1;
        if self.usage.steps > self.limits.max_steps {
            Err("max_steps")
        } else {
            Ok(())
        }
    }

    pub fn add_llm_call(&mut self, model: &str, u: &Usage) -> Result<(), String> {
        self.usage.input_tokens += u.input_tokens as u64;
        self.usage.output_tokens += u.output_tokens as u64;
        self.usage.cache_read_tokens += u.cache_read_input_tokens as u64;
        self.usage.cache_creation_tokens += u.cache_creation_input_tokens as u64;
        self.usage.reasoning_tokens += u.reasoning_tokens as u64;
        self.usage.cost_usd += ravn_llm::pricing::cost(model, u);

        if self.usage.input_tokens > self.limits.max_input_tokens {
            return Err(format!(
                "max_input_tokens ({} > {})",
                self.usage.input_tokens, self.limits.max_input_tokens
            ));
        }
        if self.usage.output_tokens > self.limits.max_output_tokens {
            return Err(format!(
                "max_output_tokens ({} > {})",
                self.usage.output_tokens, self.limits.max_output_tokens
            ));
        }
        if self.usage.cost_usd > self.limits.max_cost_usd {
            return Err(format!(
                "max_cost_usd (${:.4} > ${:.2})",
                self.usage.cost_usd, self.limits.max_cost_usd
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_cap_trips() {
        let mut t = BudgetTracker::new(Budget {
            max_steps: 2,
            ..Budget::default()
        });
        assert!(t.bump_step().is_ok());
        assert!(t.bump_step().is_ok());
        assert!(t.bump_step().is_err());
    }

    #[test]
    fn token_cap_trips_on_input() {
        let mut t = BudgetTracker::new(Budget {
            max_input_tokens: 50,
            ..Budget::default()
        });
        let u = Usage {
            input_tokens: 100,
            ..Default::default()
        };
        assert!(t.add_llm_call("claude-sonnet-4-6", &u).is_err());
    }

    #[test]
    fn cost_accumulates() {
        let mut t = BudgetTracker::new(Budget::default());
        let u = Usage {
            input_tokens: 1_000,
            output_tokens: 200,
            ..Default::default()
        };
        t.add_llm_call("claude-sonnet-4-6", &u).unwrap();
        assert!(t.usage.cost_usd > 0.0);
        assert_eq!(t.usage.input_tokens, 1_000);
    }
}
