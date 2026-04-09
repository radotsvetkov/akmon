//! Heuristic USD cost estimates from token usage (shared by TUI status bar and headless budget caps).

/// Per-million token pricing (USD).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenPricing {
    /// Input prompt tokens.
    pub input: f64,
    /// Output completion tokens.
    pub output: f64,
    /// Cache-read input tokens.
    pub cache_read: f64,
}

/// Returns per-million USD pricing for a model.
#[must_use]
pub fn pricing_for_model(model: &str) -> Option<TokenPricing> {
    let m = model.to_lowercase();
    if m.contains("ollama") {
        return Some(TokenPricing {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
        });
    }
    if m.contains("claude-haiku-4-5") || m.contains("claude-haiku-4") {
        return Some(TokenPricing {
            input: 0.80,
            output: 4.00,
            cache_read: 0.08,
        });
    }
    if m.contains("claude-sonnet-4") {
        return Some(TokenPricing {
            input: 3.00,
            output: 15.00,
            cache_read: 0.30,
        });
    }
    if m.contains("claude-opus-4") {
        return Some(TokenPricing {
            input: 15.00,
            output: 75.00,
            cache_read: 1.50,
        });
    }
    if m.starts_with("gpt-4o-mini") {
        return Some(TokenPricing {
            input: 0.15,
            output: 0.60,
            cache_read: 0.075,
        });
    }
    if m.starts_with("gpt-4o") {
        return Some(TokenPricing {
            input: 2.50,
            output: 10.00,
            cache_read: 1.25,
        });
    }
    if m.starts_with("gpt-4.1-mini") {
        return Some(TokenPricing {
            input: 0.40,
            output: 1.60,
            cache_read: 0.10,
        });
    }
    if m.starts_with("gpt-4.1") {
        return Some(TokenPricing {
            input: 2.00,
            output: 8.00,
            cache_read: 0.50,
        });
    }
    if m.starts_with("llama-3.3-70b") || m.starts_with("llama-3.1-70b") {
        return Some(TokenPricing {
            input: 0.59,
            output: 0.79,
            cache_read: 0.0,
        });
    }
    if m.starts_with("llama-3.1-8b") {
        return Some(TokenPricing {
            input: 0.05,
            output: 0.08,
            cache_read: 0.0,
        });
    }
    None
}

/// Returns estimated USD spend for a single model completion, or [`None`] when pricing is unknown.
///
/// OpenRouter-style ids (`org/model`) apply a 5.5% markup on the base lookup.
#[must_use]
pub fn estimate_cost_usd(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    model: &str,
    provider_is_openrouter: bool,
    free_local: bool,
) -> Option<f64> {
    if free_local {
        return Some(0.0);
    }
    let price = pricing_for_model(model)?;
    let mut base = (input_tokens as f64 / 1_000_000.0) * price.input
        + (output_tokens as f64 / 1_000_000.0) * price.output
        + (cache_read_tokens as f64 / 1_000_000.0) * price.cache_read;
    if provider_is_openrouter {
        base *= 1.055;
    }
    Some(base)
}
