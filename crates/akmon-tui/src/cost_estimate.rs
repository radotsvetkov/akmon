//! Heuristic USD cost estimates (delegates to [`akmon_core::estimate_cost_usd`]).

pub use akmon_core::{estimate_cost_usd, estimate_cost_usd_with_rows};

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
