//! Model-aware `max_tokens` (output limit) for chat completions.

/// Output token cap for OpenAI API–style chat models (including OpenRouter `openai/gpt-*` ids).
#[must_use]
pub fn max_tokens_for_openai_style_model(model: &str) -> u32 {
    let m = model.to_lowercase();
    if m.contains("gpt-4.1") || m.contains("gpt-5") {
        32_768
    } else if m.contains("gpt-4o")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.contains("/o1")
        || m.contains("/o3")
        || m.contains("/o4")
    {
        16_384
    } else if m.contains("gpt-4") || m.contains("gpt-3.5") {
        4_096
    } else {
        8_192
    }
}

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
    } else if m.starts_with("gpt-")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.contains("/gpt-")
        || m.contains("/o1")
        || m.contains("/o3")
        || m.contains("/o4")
    {
        max_tokens_for_openai_style_model(model)
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
    fn gpt_4_non_o_uses_turbo_cap() {
        assert_eq!(max_tokens_for_model("gpt-4-turbo"), 4_096);
    }
}
