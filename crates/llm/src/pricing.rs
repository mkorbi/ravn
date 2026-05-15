//! Pricing-Tabelle + Cost-Helper (Phase 0.10).
//!
//! Hardcoded per-million-token rates für die Modelle, die wir aktuell
//! nutzen. Quellen: Anthropic Pricing Page, OpenAI Pricing Page (Stand
//! Q2 2026). Wenn du ein neues Modell freischaltest, hier einen Eintrag
//! ergänzen — der Match ist `model_id.starts_with(prefix)`.
//!
//! `cache_read` ist 10× billiger als fresh input bei Anthropic, 50%
//! billiger bei OpenAI. `cache_creation` (Anthropic) kostet 25 % mehr
//! als fresh input.

use crate::response::Usage;

/// Per-million-token pricing in USD.
#[derive(Debug, Clone, Copy)]
pub struct Pricing {
    pub input_per_m: f64,
    pub output_per_m: f64,
    pub cache_read_per_m: f64,
    pub cache_creation_per_m: f64,
}

impl Pricing {
    pub const fn anthropic_default() -> Self {
        // Conservative fallback. Real models below.
        Self {
            input_per_m: 3.00,
            output_per_m: 15.00,
            cache_read_per_m: 0.30,
            cache_creation_per_m: 3.75,
        }
    }

    pub const fn openai_default() -> Self {
        Self {
            input_per_m: 2.50,
            output_per_m: 10.00,
            cache_read_per_m: 1.25,
            cache_creation_per_m: 2.50,
        }
    }
}

/// Look up pricing for a model id by prefix match.
pub fn lookup(model: &str) -> Pricing {
    // Anthropic Claude 4.x family (Q2 2026 pricing).
    if model.starts_with("claude-opus-4") {
        return Pricing {
            input_per_m: 15.00,
            output_per_m: 75.00,
            cache_read_per_m: 1.50,
            cache_creation_per_m: 18.75,
        };
    }
    if model.starts_with("claude-sonnet-4") {
        return Pricing {
            input_per_m: 3.00,
            output_per_m: 15.00,
            cache_read_per_m: 0.30,
            cache_creation_per_m: 3.75,
        };
    }
    if model.starts_with("claude-haiku-4") {
        return Pricing {
            input_per_m: 1.00,
            output_per_m: 5.00,
            cache_read_per_m: 0.10,
            cache_creation_per_m: 1.25,
        };
    }

    // OpenAI families.
    if model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4") {
        return Pricing {
            input_per_m: 5.00,
            output_per_m: 20.00,
            cache_read_per_m: 2.50,
            cache_creation_per_m: 5.00,
        };
    }
    if model.starts_with("gpt-4o-mini") {
        return Pricing {
            input_per_m: 0.15,
            output_per_m: 0.60,
            cache_read_per_m: 0.075,
            cache_creation_per_m: 0.15,
        };
    }
    if model.starts_with("gpt-4o") || model.starts_with("gpt-4") {
        return Pricing::openai_default();
    }

    if model.starts_with("claude-") {
        Pricing::anthropic_default()
    } else {
        Pricing::openai_default()
    }
}

/// Compute total USD cost for a single LLM call.
///
/// `usage.input_tokens` is **fresh** input only — `cache_read_input_tokens`
/// and `cache_creation_input_tokens` are billed separately and are not
/// double-counted. Adapters must populate `Usage` accordingly.
pub fn cost(model: &str, usage: &Usage) -> f64 {
    let p = lookup(model);
    let m = 1_000_000.0;
    let fresh_input = usage.input_tokens as f64 * p.input_per_m / m;
    let output = usage.output_tokens as f64 * p.output_per_m / m;
    let cache_r = usage.cache_read_input_tokens as f64 * p.cache_read_per_m / m;
    let cache_c = usage.cache_creation_input_tokens as f64 * p.cache_creation_per_m / m;
    fresh_input + output + cache_r + cache_c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sonnet_4_priced() {
        let p = lookup("claude-sonnet-4-6");
        assert_eq!(p.input_per_m, 3.00);
        assert_eq!(p.cache_read_per_m, 0.30);
    }

    #[test]
    fn cache_read_is_cheaper_than_fresh() {
        let model = "claude-sonnet-4-6";
        let fresh = Usage {
            input_tokens: 100_000,
            ..Default::default()
        };
        let cached = Usage {
            cache_read_input_tokens: 100_000,
            ..Default::default()
        };
        assert!(cost(model, &cached) * 9.0 < cost(model, &fresh));
    }

    #[test]
    fn unknown_falls_back_to_openai() {
        let p = lookup("super-future-model");
        assert_eq!(p.input_per_m, Pricing::openai_default().input_per_m);
    }
}
