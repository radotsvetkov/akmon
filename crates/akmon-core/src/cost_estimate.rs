//! Heuristic USD cost estimates from token usage (shared by TUI status bar and headless budget caps).

use serde::{Deserialize, Serialize};

/// Optional per-model hints for context-window sizing and cost math (`~/.akmon/config.toml` `[model_estimates]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCostEstimateRow {
    /// Case-insensitive substring match against the active model id (e.g. `claude-haiku-4-5`).
    pub pattern: String,
    /// Override context window used only for **UI % bars** (not sent to the API).
    #[serde(default)]
    pub context_window_tokens: Option<u64>,
    /// USD per 1M input tokens (optional override).
    #[serde(default)]
    pub input_per_million_usd: Option<f64>,
    /// USD per 1M output tokens (optional override).
    #[serde(default)]
    pub output_per_million_usd: Option<f64>,
    /// USD per 1M cache-read tokens (optional override).
    #[serde(default)]
    pub cache_read_per_million_usd: Option<f64>,
    /// Free-form note surfaced in `/context` when this row matches (e.g. rate-limit reminder).
    #[serde(default)]
    pub note: Option<String>,
}

/// First `model_estimates` row whose [`ModelCostEstimateRow::pattern`] is contained in `model` (ASCII lowercase).
#[must_use]
pub fn match_model_cost_row<'a>(
    model: &str,
    rows: &'a [ModelCostEstimateRow],
) -> Option<&'a ModelCostEstimateRow> {
    let m = model.to_lowercase();
    rows.iter().find(|r| {
        let p = r.pattern.to_lowercase();
        !p.is_empty() && m.contains(&p)
    })
}

/// Context window size used for **percentage bars only** (heuristic unless overridden in config).
#[must_use]
pub fn context_window_tokens_hint(model: &str, overrides: &[ModelCostEstimateRow]) -> u64 {
    if let Some(row) = match_model_cost_row(model, overrides) {
        if let Some(w) = row.context_window_tokens {
            return w;
        }
    }
    let m = model.to_lowercase();
    if m.contains("claude") {
        200_000
    } else if m.starts_with("gpt-4.1") {
        1_047_576
    } else if m.starts_with("gpt-4o") {
        128_000
    } else if m.starts_with("o1") || m.starts_with("o3") {
        200_000
    } else {
        8_192
    }
}

/// Built-in table lookup merged with optional `config.toml` row. Unknown models are supported only
/// when the row supplies both input and output USD rates.
#[must_use]
pub fn resolve_token_pricing_merged(
    model: &str,
    row: Option<&ModelCostEstimateRow>,
) -> Option<TokenPricing> {
    match (pricing_for_model(model), row) {
        (Some(b), Some(o)) => Some(TokenPricing {
            input: o.input_per_million_usd.unwrap_or(b.input),
            output: o.output_per_million_usd.unwrap_or(b.output),
            cache_read: o.cache_read_per_million_usd.unwrap_or(b.cache_read),
        }),
        (Some(b), None) => Some(b),
        (None, Some(o)) => {
            let input = o.input_per_million_usd?;
            let output = o.output_per_million_usd?;
            let cache_read = o.cache_read_per_million_usd.unwrap_or(0.0);
            Some(TokenPricing {
                input,
                output,
                cache_read,
            })
        }
        (None, None) => None,
    }
}

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

/// Returns estimated USD spend from resolved per-million rates.
///
/// OpenRouter-style routing applies a 5.5% markup on the computed subtotal.
#[must_use]
pub fn estimate_cost_usd_from_pricing(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    pricing: &TokenPricing,
    provider_is_openrouter: bool,
    free_local: bool,
) -> Option<f64> {
    if free_local {
        return Some(0.0);
    }
    let mut base = (input_tokens as f64 / 1_000_000.0) * pricing.input
        + (output_tokens as f64 / 1_000_000.0) * pricing.output
        + (cache_read_tokens as f64 / 1_000_000.0) * pricing.cache_read;
    if provider_is_openrouter {
        base *= 1.055;
    }
    Some(base)
}

/// Estimated USD for this usage, using built-in pricing only.
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
    estimate_cost_usd_from_pricing(
        input_tokens,
        output_tokens,
        cache_read_tokens,
        &price,
        provider_is_openrouter,
        free_local,
    )
}

/// Like [`estimate_cost_usd`] but merges `~/.akmon/config.toml` `[model_estimates]` overrides when present.
#[must_use]
pub fn estimate_cost_usd_with_rows(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    model: &str,
    provider_is_openrouter: bool,
    free_local: bool,
    model_estimates: &[ModelCostEstimateRow],
) -> Option<f64> {
    if free_local {
        return Some(0.0);
    }
    let row = match_model_cost_row(model, model_estimates);
    let pricing = resolve_token_pricing_merged(model, row)?;
    estimate_cost_usd_from_pricing(
        input_tokens,
        output_tokens,
        cache_read_tokens,
        &pricing,
        provider_is_openrouter,
        free_local,
    )
}

#[cfg(test)]
mod estimate_row_tests {
    use super::*;

    #[test]
    fn config_row_merges_partial_prices_with_builtin_table() {
        let rows = vec![ModelCostEstimateRow {
            pattern: "claude-haiku".into(),
            context_window_tokens: None,
            input_per_million_usd: Some(1.0),
            output_per_million_usd: None,
            cache_read_per_million_usd: None,
            note: None,
        }];
        let row = match_model_cost_row("claude-haiku-4-5-20251001", &rows);
        let p = resolve_token_pricing_merged("claude-haiku-4-5-20251001", row).expect("pricing");
        assert!((p.input - 1.0).abs() < f64::EPSILON);
        assert!((p.output - 4.0).abs() < 0.01);
    }

    #[test]
    fn context_window_hint_reads_override() {
        let rows = vec![ModelCostEstimateRow {
            pattern: "custom".into(),
            context_window_tokens: Some(12345),
            input_per_million_usd: None,
            output_per_million_usd: None,
            cache_read_per_million_usd: None,
            note: None,
        }];
        assert_eq!(context_window_tokens_hint("vendor/custom-7b", &rows), 12345);
    }
}
