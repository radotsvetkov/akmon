//! Heuristic USD cost estimates from token usage (for status bar display only).

/// Returns estimated USD spend for a model turn, or [`None`] when pricing is unknown.
///
/// `cache_read_tokens` are priced at 10% of the per-million input rate when the model is known.
/// OpenRouter-style ids (`org/model`) apply a 5.5% markup on the base Anthropic table lookup.
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

    let (in_per_m, out_per_m) = match () {
        _ if m.contains("opus-4") || m.contains("opus_4") => (15.0_f64, 75.0_f64),
        _ if m.contains("sonnet-4") || m.contains("sonnet_4") => (3.0_f64, 15.0_f64),
        _ if m.contains("haiku") && (m.contains("4-5") || m.contains("4_5")) => (0.8_f64, 4.0_f64),
        _ if m.contains("claude-3-5-haiku") || m.contains("3-5-haiku") => (0.8_f64, 4.0_f64),
        _ if m.contains("llama-3.3") && m.contains("70b") => (0.59_f64, 0.79_f64),
        _ if m.contains("llama-3.1") && m.contains("8b") => (0.05_f64, 0.08_f64),
        _ if m.starts_with("claude") => (3.0_f64, 15.0_f64),
        _ => return None,
    };

    let mut base = (input_tokens as f64 / 1_000_000.0) * in_per_m
        + (output_tokens as f64 / 1_000_000.0) * out_per_m
        + (cache_read_tokens as f64 / 1_000_000.0) * (in_per_m * 0.1);
    if provider_is_openrouter {
        base *= 1.055;
    }
    Some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_is_free() {
        assert_eq!(
            estimate_cost_usd(1_000_000, 1_000_000, 0, "llama3.2", false, true),
            Some(0.0)
        );
    }

    #[test]
    fn haiku_pricing_order_of_magnitude() {
        let v = estimate_cost_usd(1_000_000, 0, 0, "claude-haiku-4-5-20251001", false, false)
            .expect("known");
        assert!((v - 0.8).abs() < 0.05, "expected ~0.8/M input, got {v}");
    }

    #[test]
    fn unknown_model_none() {
        assert!(
            estimate_cost_usd(100, 100, 0, "totally-unknown-model-xyz", false, false).is_none()
        );
    }
}
