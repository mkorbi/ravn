//! Token-budget enforcement for semantic memory (Phase 1.7).
//!
//! Token counting uses a 4-chars-per-token estimate — English-biased but
//! good enough for the soft caps in [`Limits::default`]. Phase 3 swaps
//! this for `tokenizers` / `tiktoken-rs` when reasoning-router routing
//! decisions start depending on accurate counts.

use crate::semantic::SemanticMemory;

/// Per-slot and total caps. Defaults match `PLAN.md` Phase 1.7:
/// Soul ≤ 800, User ≤ 500, Total ≤ 3000 (so Memory ≤ ~1700 implicitly).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub soul_max_tokens: usize,
    pub user_max_tokens: usize,
    pub total_max_tokens: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            soul_max_tokens: 800,
            user_max_tokens: 500,
            total_max_tokens: 3000,
        }
    }
}

/// Approximate token count for `s`. 4-chars-per-token rule, rounded up.
pub fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

#[derive(Debug, Clone)]
pub struct Trimmed {
    pub memory: SemanticMemory,
    pub warnings: Vec<String>,
    pub estimated_tokens: usize,
}

/// Apply per-slot and total caps. Soul and User are trimmed in place;
/// Memory absorbs the remainder of the total budget so the most-curated
/// content (Soul, User) is preserved. Warnings are returned, not
/// printed — callers decide how to surface them (we log them in the cli
/// via tracing::warn).
pub fn enforce(mut mem: SemanticMemory, limits: &Limits) -> Trimmed {
    let mut warnings = Vec::new();

    if let Some(s) = mem.soul.as_mut() {
        if estimate_tokens(s) > limits.soul_max_tokens {
            *s = truncate_to_tokens(s, limits.soul_max_tokens);
            warnings.push(format!(
                "soul.md truncated to {}-token cap",
                limits.soul_max_tokens
            ));
        }
    }
    if let Some(u) = mem.user.as_mut() {
        if estimate_tokens(u) > limits.user_max_tokens {
            *u = truncate_to_tokens(u, limits.user_max_tokens);
            warnings.push(format!(
                "user.md truncated to {}-token cap",
                limits.user_max_tokens
            ));
        }
    }

    let used = mem.soul.as_deref().map(estimate_tokens).unwrap_or(0)
        + mem.user.as_deref().map(estimate_tokens).unwrap_or(0);
    let memory_budget = limits.total_max_tokens.saturating_sub(used);
    if let Some(m) = mem.memory.as_mut() {
        if estimate_tokens(m) > memory_budget {
            *m = truncate_to_tokens(m, memory_budget);
            warnings.push(format!(
                "memory.md truncated to {memory_budget}-token remainder of {} total cap",
                limits.total_max_tokens
            ));
        }
    }

    let estimated = mem.soul.as_deref().map(estimate_tokens).unwrap_or(0)
        + mem.memory.as_deref().map(estimate_tokens).unwrap_or(0)
        + mem.user.as_deref().map(estimate_tokens).unwrap_or(0);

    Trimmed {
        memory: mem,
        warnings,
        estimated_tokens: estimated,
    }
}

fn truncate_to_tokens(s: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(4);
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push_str("\n\n…[truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_is_chars_div_4() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("a".repeat(800).as_str()), 200);
    }

    #[test]
    fn within_caps_untouched() {
        let mem = SemanticMemory {
            soul: Some("short soul".into()),
            memory: Some("short memory".into()),
            user: Some("short user".into()),
        };
        let trimmed = enforce(mem.clone(), &Limits::default());
        assert!(trimmed.warnings.is_empty());
        assert_eq!(trimmed.memory, mem);
    }

    #[test]
    fn oversized_soul_truncated() {
        let big = "x".repeat(800 * 4 + 100);
        let mem = SemanticMemory {
            soul: Some(big),
            ..Default::default()
        };
        let trimmed = enforce(mem, &Limits::default());
        assert_eq!(trimmed.warnings.len(), 1);
        assert!(trimmed.warnings[0].contains("soul.md"));
        let soul = trimmed.memory.soul.as_deref().unwrap();
        assert!(soul.ends_with("…[truncated]"));
        assert!(estimate_tokens(soul) <= 800 + 4);
    }

    #[test]
    fn memory_absorbs_remainder_of_total_cap() {
        // Soul + User combined ~1000 tokens, leaves 2000 for memory.md.
        let soul = "s".repeat(800 * 4); // 800 tokens
        let user = "u".repeat(200 * 4); // 200 tokens
        let big_memory = "m".repeat(3000 * 4); // way over remaining 2000
        let mem = SemanticMemory {
            soul: Some(soul),
            user: Some(user),
            memory: Some(big_memory),
        };
        let trimmed = enforce(mem, &Limits::default());
        assert!(trimmed.warnings.iter().any(|w| w.contains("memory.md")));
        let mem_estimated = estimate_tokens(trimmed.memory.memory.as_deref().unwrap());
        // Remaining budget = 3000 - 800 - 200 = 2000.
        // Allow a couple-token slack for the …[truncated] marker.
        assert!(mem_estimated <= 2000 + 10);
    }

    #[test]
    fn empty_memory_no_warnings() {
        let mem = SemanticMemory::default();
        let trimmed = enforce(mem, &Limits::default());
        assert!(trimmed.warnings.is_empty());
        assert_eq!(trimmed.estimated_tokens, 0);
    }
}
