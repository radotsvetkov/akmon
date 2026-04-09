//! Heuristic USD cost estimates from token usage (shared by TUI status bar and headless budget caps).

/// Returns estimated USD spend for a single model completion, or [`None`] when pricing is unknown.
///
/// `cache_read_tokens` are priced at a fraction of the per-million input rate when the model is known.
/// OpenRouter-style ids (`org/model`) apply a 5.5% markup on the base Anthropic table lookup.
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
    let m = model.to_lowercase();
    if m.contains("ollama") {
        return Some(0.0);
    }

    let (in_per_m, out_per_m, cache_frac_input) = match () {
        _ if m.contains("opus-4") || m.contains("opus_4") => (15.0_f64, 75.0_f64, 0.1_f64),
        _ if m.contains("sonnet-4") || m.contains("sonnet_4") => (3.0_f64, 15.0_f64, 0.1_f64),
        _ if m.contains("haiku") && (m.contains("4-5") || m.contains("4_5")) => {
            (0.8_f64, 4.0_f64, 0.1_f64)
        }
        _ if m.contains("claude-3-5-haiku") || m.contains("3-5-haiku") => {
            (0.8_f64, 4.0_f64, 0.1_f64)
        }
        _ if m.contains("gpt-4o-mini") => (0.15_f64, 0.60_f64, 0.5_f64),
        _ if m.contains("gpt-4.1-mini") => (0.40_f64, 1.60_f64, 0.25_f64),
        _ if m.contains("gpt-4.1") => (2.00_f64, 8.0_f64, 0.25_f64),
        _ if m.contains("gpt-5") => (2.00_f64, 8.0_f64, 0.25_f64),
        _ if m.contains("gpt-4o") => (2.50_f64, 10.0_f64, 0.5_f64),
        _ if m.contains("gpt-4-") || m.contains("gpt-4-turbo") => {
            (10.0_f64, 30.0_f64, 0.5_f64)
        }
        _ if m.contains("gpt-4") => (2.50_f64, 10.0_f64, 0.5_f64),
        _ if m.contains("gpt-3.5") => (0.50_f64, 1.50_f64, 0.5_f64),
        _ if m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") => {
            (15.0_f64, 60.0_f64, 0.5_f64)
        }
        _ if m.contains("llama-3.3") && m.contains("70b") => (0.59_f64, 0.79_f64, 0.0_f64),
        _ if m.contains("llama-3.1") && m.contains("8b") => (0.05_f64, 0.08_f64, 0.0_f64),
        _ if m.starts_with("mixtral-8x7b") => (0.24_f64, 0.24_f64, 0.0_f64),
        _ if m.starts_with("claude") => (3.0_f64, 15.0_f64, 0.1_f64),
        _ => return None,
    };

    let mut base = (input_tokens as f64 / 1_000_000.0) * in_per_m
        + (output_tokens as f64 / 1_000_000.0) * out_per_m
        + (cache_read_tokens as f64 / 1_000_000.0) * (in_per_m * cache_frac_input);
    if provider_is_openrouter {
        base *= 1.055;
    }
    Some(base)
}
