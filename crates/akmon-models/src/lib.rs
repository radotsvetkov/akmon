//! Model backends and the [`LlmProvider`] abstraction (Ollama, Anthropic, …).

#![warn(missing_docs)]

mod anthropic;
mod config;
mod error;
mod message;
mod ollama;
mod stream;
mod tool_def;

pub use anthropic::{
    AnthropicBackend, DEFAULT_ANTHROPIC_BASE_URL, DEFAULT_ANTHROPIC_CONTEXT_WINDOW,
    DEFAULT_ANTHROPIC_MODEL,
};
pub use config::CompletionConfig;
pub use error::ModelError;
pub use message::{Message, MessageRole};
pub use ollama::OllamaBackend;
pub use stream::{CompletionStream, ModelToolCall, StopReason, StreamEvent};
pub use tool_def::ToolDefinition;

use async_trait::async_trait;

/// Pluggable large-language-model backend (Ollama today; cloud APIs later).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Short provider id (e.g. `ollama`), stable for logs and config keys.
    fn name(&self) -> &str;

    /// Advertised context window size in tokens (best-effort for local servers).
    fn context_window_tokens(&self) -> usize;

    /// Runs one completion. Yields [`StreamEvent`] values until [`StreamEvent::Done`] or [`StreamEvent::Error`].
    ///
    /// HTTP timeouts and [`CompletionConfig::first_token_deadline_ms`] are enforced by each backend.
    async fn complete(
        &self,
        messages: &[Message],
        config: &CompletionConfig,
    ) -> Result<CompletionStream, ModelError>;
}
