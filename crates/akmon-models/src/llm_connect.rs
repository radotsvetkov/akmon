//! Configuration bundle for [`crate::LlmProvider`] selection (multi-cloud detection).

use std::sync::Arc;

use akmon_core::Secret;

use crate::{
    AnthropicBackend, BedrockBackend, LlmProvider, OllamaBackend, OpenAiCompatBackend,
    ProviderError,
};

pub(crate) fn nonempty(opt: Option<String>) -> Option<String> {
    opt.filter(|s| !s.trim().is_empty())
}

/// Anthropic API model ids (`claude-…`); checked before Ollama heuristics so `claude-*` never routes to Ollama.
pub fn looks_like_claude_api_model(model: &str) -> bool {
    let lower = model.trim().to_lowercase();
    lower.starts_with("claude-") || lower == "claude"
}

pub(crate) fn looks_like_openai_chat_model(model: &str) -> bool {
    let lower = model.trim().to_lowercase();
    lower.starts_with("gpt-")
        || lower.starts_with("chatgpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
}

pub(crate) fn looks_like_groq_hosted_model(model: &str) -> bool {
    let lower = model.trim().to_lowercase();
    lower.starts_with("llama") || lower.starts_with("mixtral")
}

/// Heuristic for Ollama-style local model ids (`:` tags or common local prefixes).
pub fn looks_like_ollama_model(model: &str) -> bool {
    let lower = model.trim().to_lowercase();
    if looks_like_claude_api_model(model) {
        return false;
    }
    if model.contains(':') {
        return true;
    }
    const PREFIXES: &[&str] = &[
        "llama",
        "qwen",
        "mistral",
        "deepseek",
        "codellama",
        "phi",
        "gemma",
        "vicuna",
        "orca",
        "neural",
        "wizardcoder",
        "starcoder",
        "tinyllama",
        "falcon",
        "dolphin",
    ];
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// Provider display label inferred from model id only (UI hint, no key checks).
#[must_use]
pub fn provider_display_name(model: &str) -> &'static str {
    if looks_like_claude_api_model(model) {
        return "Anthropic";
    }
    if model.contains('/') {
        return "OpenRouter";
    }
    let lower = model.trim().to_lowercase();
    if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        return "OpenAI";
    }
    if lower.starts_with("llama-") || lower.starts_with("mixtral-") {
        return "Groq";
    }
    "Ollama"
}

/// CLI / config inputs that determine which [`LlmProvider`] implementation to use.
#[derive(Debug, Clone)]
pub struct LlmConnectConfig {
    /// Model id (Ollama tag, Claude id, OpenRouter `org/model`, Bedrock model id, …).
    pub model: String,
    /// Ollama API base when falling back to local inference.
    pub ollama_url: String,
    /// Anthropic API key (optional).
    pub anthropic_api_key: Option<String>,
    /// OpenRouter API key (optional).
    pub openrouter_api_key: Option<String>,
    /// OpenAI API key (optional).
    pub openai_api_key: Option<String>,
    /// Groq API key (optional).
    pub groq_api_key: Option<String>,
    /// Azure OpenAI resource URL ending in `/deployments/{name}/chat/completions`.
    pub azure_openai_endpoint: Option<String>,
    /// Azure OpenAI subscription key (sent as `api-key`, not Bearer).
    pub azure_openai_api_key: Option<String>,
    /// Query `api-version` for Azure (e.g. `2024-02-01`).
    pub azure_api_version: String,
    /// Explicit `--bedrock` (also triggers when `AWS_ACCESS_KEY_ID` is set — see [`Self::resolve`]).
    pub bedrock_explicit: bool,
    /// AWS region for Bedrock.
    pub aws_region: String,
    /// Generic OpenAI-compatible base URL (no `/chat/completions` suffix).
    pub openai_compatible_url: Option<String>,
    /// API key for [`Self::openai_compatible_url`] when the server requires auth.
    pub openai_compatible_api_key: Option<String>,
}

impl Default for LlmConnectConfig {
    fn default() -> Self {
        Self {
            model: "llama3.2".into(),
            ollama_url: "http://localhost:11434".into(),
            anthropic_api_key: None,
            openrouter_api_key: None,
            openai_api_key: None,
            groq_api_key: None,
            azure_openai_endpoint: None,
            azure_openai_api_key: None,
            azure_api_version: "2024-02-01".into(),
            bedrock_explicit: false,
            aws_region: "us-east-1".into(),
            openai_compatible_url: None,
            openai_compatible_api_key: None,
        }
    }
}

impl LlmConnectConfig {
    /// Deterministic provider resolution trace (calls [`Self::resolve`] once; introspection only).
    #[must_use]
    pub fn explain_provider_resolution(&self) -> crate::ProviderResolutionTrace {
        crate::provider_resolution::explain_provider_resolution(self)
    }

    /// Human-readable backend label for status bars (matches [`Self::resolve`] priority).
    pub fn inferred_backend_name(&self) -> &'static str {
        let model = self.model.trim();

        if looks_like_claude_api_model(model) {
            return "Anthropic";
        }
        let aws_key_present = std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
        if self.bedrock_explicit || aws_key_present {
            return "AWS Bedrock";
        }

        if model.contains('/')
            && self
                .openrouter_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return "OpenRouter";
        }

        if self
            .azure_openai_endpoint
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && self
                .azure_openai_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return "Azure OpenAI";
        }

        if self
            .openai_api_key
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && looks_like_openai_chat_model(model)
        {
            return "OpenAI";
        }

        if self
            .groq_api_key
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && looks_like_groq_hosted_model(model)
        {
            return "Groq";
        }

        if self
            .openai_compatible_url
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
            && self
                .openai_compatible_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
        {
            return "OpenAI-compatible";
        }

        if looks_like_ollama_model(model) {
            return "Ollama";
        }

        "Ollama"
    }

    /// Resolves a provider using priority order: Bedrock → native `claude-*` (Anthropic / OpenRouter) → OpenRouter (`org/model`) → Azure → OpenAI (gpt/o\* only) → Groq (llama/mixtral only) → custom URL → Ollama heuristics → Ollama default.
    pub fn resolve(self) -> Result<Arc<dyn LlmProvider>, ProviderError> {
        let model = self.model;
        let model_trim = model.trim();
        let aws_key_present = std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
        if self.bedrock_explicit || aws_key_present {
            return match BedrockBackend::from_env(self.aws_region, model.clone()) {
                Some(b) => Ok(Arc::new(b)),
                None => Err(ProviderError::AwsCredentialsNotFound),
            };
        }

        // Native Anthropic API ids (`claude-haiku-…`) before OpenRouter slash slugs and Ollama.
        if looks_like_claude_api_model(model_trim) && !model_trim.contains('/') {
            if let Some(key) = nonempty(self.anthropic_api_key) {
                return Ok(Arc::new(AnthropicBackend::new(Secret::new(key), model)));
            }
            if let Some(key) = nonempty(self.openrouter_api_key) {
                let or_id = format!("anthropic/{model_trim}");
                return Ok(Arc::new(OpenAiCompatBackend::openrouter(key, or_id)));
            }
            return Err(ProviderError::ClaudeModelsRequireApiKey);
        }

        if model_trim.contains('/') {
            let key = nonempty(self.openrouter_api_key)
                .ok_or(ProviderError::OpenRouterKeyRequiredForSlashModel)?;
            return Ok(Arc::new(OpenAiCompatBackend::openrouter(key, model)));
        }

        if let (Some(ep), Some(k)) = (
            nonempty(self.azure_openai_endpoint),
            nonempty(self.azure_openai_api_key),
        ) {
            return Ok(Arc::new(OpenAiCompatBackend::azure(
                ep,
                k,
                self.azure_api_version,
            )));
        }

        if let Some(key) = nonempty(self.openai_api_key)
            && looks_like_openai_chat_model(model_trim)
        {
            return Ok(Arc::new(OpenAiCompatBackend::openai(key, model)));
        }

        if let Some(key) = nonempty(self.groq_api_key)
            && looks_like_groq_hosted_model(model_trim)
        {
            return Ok(Arc::new(OpenAiCompatBackend::groq(key, model)));
        }

        if let Some(url) = nonempty(self.openai_compatible_url) {
            let key = nonempty(self.openai_compatible_api_key)
                .ok_or(ProviderError::OpenAiCompatibleKeyRequired)?;
            return Ok(Arc::new(OpenAiCompatBackend::custom(url, key, model)));
        }

        if looks_like_ollama_model(&model) {
            return Ok(Arc::new(OllamaBackend::new(self.ollama_url, model)));
        }

        Ok(Arc::new(OllamaBackend::new(self.ollama_url, model)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_model_requires_openrouter_key() {
        let r = LlmConnectConfig {
            model: "anthropic/claude-3.5-haiku".into(),
            ..Default::default()
        }
        .resolve();
        match r {
            Err(e) => assert_eq!(e, ProviderError::OpenRouterKeyRequiredForSlashModel),
            Ok(p) => panic!("expected OpenRouter key error, got provider {}", p.name()),
        }
    }

    #[test]
    fn slash_model_with_key_is_openrouter() {
        let r = LlmConnectConfig {
            model: "anthropic/claude-3.5-haiku".into(),
            openrouter_api_key: Some("sk-or".into()),
            ..Default::default()
        }
        .resolve();
        assert!(r.is_ok());
        let p = r.expect("ok");
        assert!(p.name().starts_with("openrouter/"));
    }

    #[test]
    fn openai_key_selects_openai() {
        if std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            return;
        }
        let r = LlmConnectConfig {
            model: "gpt-4o".into(),
            openai_api_key: Some("sk-openai".into()),
            ..Default::default()
        }
        .resolve()
        .expect("provider");
        assert!(r.name().starts_with("openai"));
    }

    #[test]
    fn no_keys_falls_back_ollama() {
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok() {
            return;
        }
        let r = LlmConnectConfig {
            model: "llama3.2".into(),
            ..Default::default()
        }
        .resolve()
        .expect("ollama");
        assert_eq!(r.name(), "ollama");
    }

    #[test]
    fn claude_model_without_keys_errors_not_ollama() {
        if std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            return;
        }
        let r = LlmConnectConfig {
            model: "claude-haiku-4-5-20251001".into(),
            ..Default::default()
        }
        .resolve();
        let e = match r {
            Err(e) => e,
            Ok(p) => panic!("expected configuration error, got provider {}", p.name()),
        };
        assert_eq!(e, ProviderError::ClaudeModelsRequireApiKey);
        let msg = e.to_string();
        assert!(
            msg.contains("ANTHROPIC") && msg.contains("OPENROUTER"),
            "{msg}"
        );
        assert!(
            !msg.to_lowercase().contains("ollama"),
            "must not route Claude to Ollama: {msg}"
        );
    }

    #[test]
    fn llama_with_only_openai_key_uses_ollama_not_openai() {
        if std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            return;
        }
        let r = LlmConnectConfig {
            model: "llama3.2".into(),
            openai_api_key: Some("sk-openai".into()),
            ..Default::default()
        }
        .resolve()
        .expect("ollama");
        assert_eq!(r.name(), "ollama");
    }

    #[test]
    fn bedrock_flag_without_env_errors() {
        let r = LlmConnectConfig {
            model: "anthropic.claude-3-5-haiku-20241022-v1:0".into(),
            bedrock_explicit: true,
            ..Default::default()
        }
        .resolve();
        match r {
            Err(e) => assert_eq!(e, ProviderError::AwsCredentialsNotFound),
            Ok(p) => panic!("expected AWS credentials error, got provider {}", p.name()),
        }
    }
}
