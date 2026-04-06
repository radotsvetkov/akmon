//! Model backends and the [`LlmProvider`] abstraction (Ollama, Anthropic, …).

#![warn(missing_docs)]

mod anthropic;
mod bedrock;
mod config;
mod error;
mod llm_connect;
mod message;
mod ollama;
mod openai_compat;
mod stream;
mod tool_def;

pub use anthropic::{
    anthropic_system_block_text, AnthropicBackend, DEFAULT_ANTHROPIC_BASE_URL,
    DEFAULT_ANTHROPIC_CONTEXT_WINDOW, DEFAULT_ANTHROPIC_MODEL,
};
pub use bedrock::{BedrockBackend, BEDROCK_DISPLAY_MODEL_IDS};
pub use config::CompletionConfig;
pub use error::ModelError;
pub use llm_connect::LlmConnectConfig;
pub use message::{Message, MessageRole};
pub use ollama::OllamaBackend;
pub use openai_compat::{infer_context_window_tokens, OpenAiCompatBackend};
pub use stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent, UsageReport};
pub use tool_def::ToolDefinition;

use async_trait::async_trait;

/// Conservative token approximation when no backend tokenizer is available. Uses `ceil(chars / 3.5)`
/// rounded up to reduce the risk of silent context overflow. Slightly overestimates.
pub fn approximate_tokens(messages: &[Message]) -> usize {
    let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    ((total_chars as f64) / 3.5).ceil() as usize
}

/// Pluggable large-language-model backend (Ollama today; cloud APIs later).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Short provider id (e.g. `ollama`), stable for logs and config keys.
    fn name(&self) -> &str;

    /// Advertised context window size in tokens (best-effort for local servers).
    fn context_window_tokens(&self) -> usize;

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
        assert_eq!(approximate_tokens(&m), 2);
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
        assert_eq!(approximate_tokens(&m), 1);
    }
}
