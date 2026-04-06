//! Model-aware `max_tokens` (output limit) for chat completions.

/// Returns a reasonable maximum **output** token budget for the given model id.
///
/// Used for API `max_tokens` / `num_predict`; keep values within each provider’s caps.
#[must_use]
pub fn max_tokens_for_model(model: &str) -> u32 {
    let m = model.to_lowercase();
    if m.contains("haiku") {
        8_192
    } else if m.contains("sonnet") || m.contains("opus") {
        32_000
    } else if m.contains("gpt-4o") {
        16_384
    } else {
        8_192
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haiku_budget() {
        assert_eq!(max_tokens_for_model("claude-haiku-4-5-20251001"), 8_192);
    }

    #[test]
    fn sonnet_budget() {
        assert_eq!(max_tokens_for_model("claude-sonnet-foo"), 32_000);
    }

    #[test]
    fn opus_budget() {
        assert_eq!(max_tokens_for_model("claude-opus-4"), 32_000);
    }

    #[test]
    fn default_budget() {
        assert_eq!(max_tokens_for_model("llama3.2"), 8_192);
    }

    #[test]
    fn gpt_4o_budget() {
        assert_eq!(max_tokens_for_model("gpt-4o-mini"), 16_384);
    }

    #[test]
    fn gpt_4_non_o_uses_default() {
        assert_eq!(max_tokens_for_model("gpt-4-turbo"), 8_192);
    }
}
