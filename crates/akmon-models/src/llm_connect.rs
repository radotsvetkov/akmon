//! Configuration bundle for [`crate::LlmProvider`] selection (multi-cloud detection).

use std::sync::Arc;

use akmon_core::Secret;

use crate::{AnthropicBackend, BedrockBackend, LlmProvider, OllamaBackend, OpenAiCompatBackend};

fn nonempty(opt: Option<String>) -> Option<String> {
    opt.filter(|s| !s.trim().is_empty())
}

/// Heuristic for Ollama-style local model ids (`:` tags or common local prefixes).
pub fn looks_like_ollama_model(model: &str) -> bool {
    if model.contains(':') {
        return true;
    }
    let lower = model.to_lowercase();
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
    /// Human-readable backend label for status bars (matches [`Self::resolve`] priority).
    pub fn inferred_backend_name(&self) -> &'static str {
        let model = self.model.trim();
        let lower = model.to_lowercase();
        // Claude ids are always Anthropic-class in the product UI, even if routing falls through.
        if lower.starts_with("claude") {
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
        {
            return "OpenAI";
        }

        if self
            .groq_api_key
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
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

    /// Resolves a provider using priority order: Bedrock → OpenRouter (`org/model`) → Anthropic (Claude + key) → Claude via OpenRouter → Azure → OpenAI → Groq → custom URL → Ollama heuristics → Ollama default.
    pub fn resolve(self) -> Result<Arc<dyn LlmProvider>, String> {
        let model = self.model;
        let aws_key_present = std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
        if self.bedrock_explicit || aws_key_present {
            return match BedrockBackend::from_env(self.aws_region, model.clone()) {
                Some(b) => Ok(Arc::new(b)),
                None => Err(String::from(
                    "AWS credentials not found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY env vars.",
                )),
            };
        }

        if model.contains('/') {
            let key = nonempty(self.openrouter_api_key).ok_or_else(|| {
                String::from(
                    "OpenRouter model id contains '/' — set OPENROUTER_API_KEY or --openrouter-key.",
                )
            })?;
            return Ok(Arc::new(OpenAiCompatBackend::openrouter(key, model)));
        }

        let lower = model.to_lowercase();
        if lower.starts_with("claude") {
            if let Some(key) = nonempty(self.anthropic_api_key) {
                return Ok(Arc::new(AnthropicBackend::new(Secret::new(key), model)));
            }
            if let Some(key) = nonempty(self.openrouter_api_key) {
                let or_id = format!("anthropic/{model}");
                return Ok(Arc::new(OpenAiCompatBackend::openrouter(key, or_id)));
            }
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

        if let Some(key) = nonempty(self.openai_api_key) {
            return Ok(Arc::new(OpenAiCompatBackend::openai(key, model)));
        }

        if let Some(key) = nonempty(self.groq_api_key) {
            return Ok(Arc::new(OpenAiCompatBackend::groq(key, model)));
        }

        if let Some(url) = nonempty(self.openai_compatible_url) {
            let key = nonempty(self.openai_compatible_api_key).ok_or_else(|| {
                String::from(
                    "Set --openai-compatible-key (or config) for the custom OpenAI-compatible URL.",
                )
            })?;
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
        assert!(r.is_err());
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
    fn bedrock_flag_without_env_errors() {
        let r = LlmConnectConfig {
            model: "anthropic.claude-3-5-haiku-20241022-v1:0".into(),
            bedrock_explicit: true,
            ..Default::default()
        }
        .resolve();
        assert!(r.is_err());
    }
}
