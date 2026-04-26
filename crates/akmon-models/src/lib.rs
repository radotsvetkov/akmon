//! Model backends and the [`LlmProvider`] abstraction (Ollama, Anthropic, …).

#![warn(missing_docs)]

mod anthropic;
mod bedrock;
mod config;
mod error;
pub mod journaling;
mod llm_connect;
mod max_tokens;
mod message;
mod ollama;
mod ollama_models;
mod openai_compat;
mod provider_error;
mod provider_resolution;
mod stream;
mod tool_def;

pub use anthropic::{
    AnthropicBackend, DEFAULT_ANTHROPIC_BASE_URL, DEFAULT_ANTHROPIC_CONTEXT_WINDOW,
    DEFAULT_ANTHROPIC_MODEL, anthropic_system_block_text,
};
pub use bedrock::{BEDROCK_DISPLAY_MODEL_IDS, BedrockBackend};
pub use config::CompletionConfig;
pub use error::ModelError;
pub use llm_connect::{
    LlmConnectConfig, looks_like_claude_api_model, looks_like_ollama_model, provider_display_name,
};
pub use max_tokens::{max_tokens_for_model, max_tokens_for_openai_style_model};
pub use message::{Message, MessageRole};
pub use ollama::OllamaBackend;
pub use ollama_models::{
    OllamaCapabilityHint, OllamaModel, OllamaProbe, fetch_ollama_models,
    infer_ollama_capability_hint, ollama_first_token_deadline_ms, ollama_stream_idle_timeout_secs,
    probe_ollama,
};
pub use openai_compat::{OpenAiCompatBackend, infer_context_window_tokens};
pub use provider_error::{ProviderError, ProviderResult};
pub use provider_resolution::{ProviderResolutionCandidate, ProviderResolutionTrace};
pub use stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent, UsageReport};
pub use tool_def::ToolDefinition;

use async_trait::async_trait;

/// Heuristic token estimate for a single string (prose vs code-ish lines).
#[must_use]
pub fn estimate_tokens_for_content(content: &str) -> usize {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return 0;
    }
    let code_line_count = lines
        .iter()
        .filter(|l| {
            let t = l.trim();
            t.ends_with(';')
                || t.ends_with('{')
                || t.ends_with('}')
                || t.contains("fn ")
                || t.contains("let ")
                || t.contains("def ")
                || t.contains("class ")
                || t.contains("import ")
                || t.contains("    ")
        })
        .count();
    let code_ratio = code_line_count as f64 / lines.len() as f64;
    let tokens = if code_ratio > 0.4 {
        (lines.len() as f64 * 18.0).ceil() as u64
    } else {
        let words = content.split_whitespace().count();
        (words as f64 * 1.3).ceil() as u64
    };
    ((tokens * 4) / 3) as usize
}

/// Sums [`estimate_tokens_for_content`] across all message bodies.
#[must_use]
pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens_for_content(&m.content))
        .sum()
}

/// Conservative token approximation when no backend tokenizer is available.
#[must_use]
pub fn approximate_tokens(messages: &[Message]) -> usize {
    estimate_messages_tokens(messages)
}

/// Pluggable large-language-model backend (Ollama today; cloud APIs later).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Short provider id (e.g. `ollama`), stable for logs and config keys.
    fn name(&self) -> &str;

    /// Advertised context window size in tokens (best-effort for local servers).
    fn context_window_tokens(&self) -> usize;

    /// API model id string (e.g. `claude-haiku-4-5-20251001`, Ollama tag, OpenRouter slug).
    ///
    /// Used with [`crate::max_tokens_for_model`] to set per-request output limits.
    fn completion_model_id(&self) -> &str;

    /// Estimate the token count for a slice of messages.
    ///
    /// Returns [`None`] if the backend does not provide a tokenizer — callers use
    /// [`approximate_tokens`] instead.
    fn estimate_tokens(&self, messages: &[Message]) -> Option<usize> {
        let _ = messages;
        None
    }

    /// Runs one completion. Yields [`StreamEvent`] values until [`StreamEvent::Done`] or [`StreamEvent::Error`].
    ///
    /// HTTP timeouts and [`CompletionConfig::first_token_deadline_ms`] are enforced by each backend.
    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError>;
}

#[cfg(test)]
mod approximate_token_tests {
    use super::*;

    #[test]
    fn approximate_tokens_empty_is_zero() {
        assert_eq!(approximate_tokens(&[]), 0);
    }

    #[test]
    fn approximate_tokens_seven_chars_ceil() {
        let m = vec![Message {
            role: MessageRole::User,
            content: "1234567".into(),
        }];
        let n = approximate_tokens(&m);
        assert!((1..=4).contains(&n), "unexpected estimate {n}");
    }

    #[test]
    fn approximate_tokens_sums_across_messages() {
        let m = vec![
            Message {
                role: MessageRole::User,
                content: "a".into(),
            },
            Message {
                role: MessageRole::Assistant,
                content: "bb".into(),
            },
        ];
        let n = approximate_tokens(&m);
        assert!(
            n >= 2,
            "expected >1 token est for two short messages, got {n}"
        );
    }
}
