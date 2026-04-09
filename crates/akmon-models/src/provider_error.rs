//! Errors when selecting or configuring an [`crate::LlmProvider`] before any request is sent.

use thiserror::Error;

/// Failure to build a provider from [`crate::LlmConnectConfig::resolve`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    /// Bedrock was selected but AWS credentials could not be loaded.
    #[error("AWS credentials not found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY env vars.")]
    AwsCredentialsNotFound,

    /// A native `claude-*` model id was given without Anthropic or OpenRouter credentials.
    #[error(
        "Claude models require ANTHROPIC_API_KEY or OPENROUTER_API_KEY (set env or use --anthropic-key / --openrouter-key)."
    )]
    ClaudeModelsRequireApiKey,

    /// An `org/model` slug was used without an OpenRouter API key.
    #[error("OpenRouter model id contains '/' — set OPENROUTER_API_KEY or --openrouter-key.")]
    OpenRouterKeyRequiredForSlashModel,

    /// Custom OpenAI-compatible URL was set without an API key.
    #[error("Set --openai-compatible-key (or config) for the custom OpenAI-compatible URL.")]
    OpenAiCompatibleKeyRequired,
}

/// [`Result`] alias for provider wiring.
pub type ProviderResult<T> = Result<T, ProviderError>;
