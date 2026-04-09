//! Per-completion tuning for model calls.

use crate::max_tokens_for_model;
use crate::tool_def::ToolDefinition;

/// Parameters for a single [`crate::LlmProvider::complete`] invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionConfig {
    /// Maximum number of tokens the model may generate for this completion.
    pub max_tokens: u32,
    /// Optional session id (e.g. OpenRouter `prompt_cache_key` for sticky routing).
    pub session_id: Option<String>,
    /// Sampling temperature (higher = more random).
    pub temperature: f32,
    /// Wall-clock budget from request start until the **first** streamed chunk must arrive.
    ///
    /// If the deadline passes with no data, [`crate::ModelError::FirstTokenTimeout`] is returned.
    pub first_token_deadline_ms: u64,
    /// When `true`, use the provider's streaming API; when `false`, buffer one response.
    pub stream: bool,
    /// Tool definitions passed to backends that support function calling (e.g. Ollama `tools`).
    pub tools: Vec<ToolDefinition>,
    /// On repeated HTTP 429/529 responses, later retries may use this model id instead (Anthropic / OpenAI-compatible).
    pub fallback_model: Option<String>,
}

impl Default for CompletionConfig {
    /// Defaults: `max_tokens` 8192 (see [`crate::max_tokens_for_model`] for model-specific values),
    /// `temperature` 0.7, `first_token_deadline_ms` 5000, `stream` true.
    fn default() -> Self {
        Self {
            max_tokens: max_tokens_for_model(""),
            session_id: None,
            temperature: 0.7,
            first_token_deadline_ms: 5000,
            stream: true,
            tools: Vec::new(),
            fallback_model: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_temperature_deadline_stream() {
        let c = CompletionConfig::default();
        assert_eq!(c.max_tokens, 8192);
        assert_eq!(c.temperature, 0.7);
        assert_eq!(c.first_token_deadline_ms, 5000);
        assert!(c.stream);
        assert!(c.tools.is_empty());
    }
}
