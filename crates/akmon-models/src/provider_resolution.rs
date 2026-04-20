//! Deterministic provider resolution trace for [`crate::LlmConnectConfig`] (introspection only).

use std::sync::Arc;

use crate::LlmProvider;
use crate::bedrock::BedrockBackend;
use crate::llm_connect::{
    LlmConnectConfig, looks_like_claude_api_model, looks_like_groq_hosted_model,
    looks_like_ollama_model, looks_like_openai_chat_model, nonempty,
};
use crate::provider_error::ProviderError;

/// Full provider routing explanation (matches [`LlmConnectConfig::resolve`] outcomes; additive only).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProviderResolutionTrace {
    /// Stable id when resolution succeeds (`anthropic`, `openrouter`, `ollama`, …).
    pub selected_provider: Option<String>,
    /// Human-readable summary of why this route won or why resolution failed.
    pub selected_reason: String,
    /// Model id string used for routing (trimmed copy of config).
    pub model_id: String,
    /// Present when [`crate::LlmConnectConfig::resolve`] returns `Err` (same variant).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_error: Option<ProviderError>,
    /// Candidates in resolver priority order (lower [`ProviderResolutionCandidate::priority_order`] is tried first).
    pub candidates: Vec<ProviderResolutionCandidate>,
}

/// One candidate route in [`ProviderResolutionTrace`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProviderResolutionCandidate {
    /// Short stable name (`bedrock`, `native_claude`, `openrouter`, …).
    pub provider: String,
    /// `true` only for the winning route on success.
    pub eligible: bool,
    /// Why this route was selected, skipped, ineligible, or not evaluated.
    pub reason: String,
    /// Named prerequisites that are missing or misconfigured (never secret values).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_prerequisites: Vec<String>,
    /// Matches [`crate::LlmConnectConfig::resolve`] branch order (1 = first).
    pub priority_order: u32,
}

/// Build a trace for the current config; calls [`LlmConnectConfig::resolve`] once for authoritative selection.
pub fn explain_provider_resolution(cfg: &LlmConnectConfig) -> ProviderResolutionTrace {
    let model_id = cfg.model.trim().to_string();
    let resolved = cfg.clone().resolve();
    let selected_provider = resolved
        .as_ref()
        .ok()
        .map(|p| canonical_provider_id(p.as_ref()));
    let resolution_error = resolved.as_ref().err().cloned();
    let selected_reason = describe_selected_reason(cfg, &resolved);
    let candidates = build_candidates(cfg, &resolved);
    ProviderResolutionTrace {
        selected_provider,
        selected_reason,
        model_id,
        resolution_error,
        candidates,
    }
}

/// Maps a built provider to a short stable id (for traces and parity checks).
pub fn canonical_provider_id(p: &dyn LlmProvider) -> String {
    let n = p.name();
    if n == "anthropic" {
        return "anthropic".into();
    }
    if n == "ollama" {
        return "ollama".into();
    }
    if n.starts_with("bedrock/") {
        return "bedrock".into();
    }
    if n.starts_with("openrouter/") {
        return "openrouter".into();
    }
    if n.starts_with("azure/") {
        return "azure_openai".into();
    }
    if n.starts_with("groq/") {
        return "groq".into();
    }
    if n.starts_with("openai/") {
        return "openai".into();
    }
    "openai_compatible".into()
}

fn aws_access_key_nonempty() -> bool {
    std::env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
}

fn bedrock_branch_active(cfg: &LlmConnectConfig) -> bool {
    cfg.bedrock_explicit || aws_access_key_nonempty()
}

fn failure_step(err: &ProviderError) -> u32 {
    match err {
        ProviderError::AwsCredentialsNotFound => 1,
        ProviderError::ClaudeModelsRequireApiKey => 2,
        ProviderError::OpenRouterKeyRequiredForSlashModel => 3,
        ProviderError::OpenAiCompatibleKeyRequired => 7,
    }
}

fn success_step(cfg: &LlmConnectConfig, p: &dyn LlmProvider) -> u32 {
    let n = p.name();
    if n.starts_with("bedrock/") {
        return 1;
    }
    if n == "anthropic" {
        return 2;
    }
    if n.starts_with("openrouter/") {
        let m = cfg.model.trim();
        if looks_like_claude_api_model(m) && !m.contains('/') {
            return 2;
        }
        return 3;
    }
    if n.starts_with("azure/") {
        return 4;
    }
    if n.starts_with("openai/") {
        return 5;
    }
    if n.starts_with("groq/") {
        return 6;
    }
    if n == "ollama" {
        if looks_like_ollama_model(&cfg.model) {
            return 8;
        }
        return 9;
    }
    7
}

fn describe_selected_reason(
    cfg: &LlmConnectConfig,
    resolved: &Result<Arc<dyn LlmProvider>, ProviderError>,
) -> String {
    let _ = cfg;
    match resolved {
        Ok(p) => {
            let id = canonical_provider_id(p.as_ref());
            format!(
                "Resolution succeeded: selected provider `{id}` (same outcome as `LlmConnectConfig::resolve`)."
            )
        }
        Err(e) => format!("Resolution failed: {e} (same error as `LlmConnectConfig::resolve`)."),
    }
}

/// `win` / `fail` are resolver outcome steps (see `success_step` / `failure_step`).
fn row_state(step: u32, win: Option<u32>, fail: Option<u32>) -> &'static str {
    if let Some(w) = win {
        if step < w {
            return "superseded";
        }
        if step == w {
            return "matched";
        }
        return "not_evaluated";
    }
    if let Some(f) = fail {
        if step < f {
            return "skipped";
        }
        if step == f {
            return "failed_here";
        }
        return "not_evaluated";
    }
    "not_evaluated"
}

fn build_candidates(
    cfg: &LlmConnectConfig,
    resolved: &Result<Arc<dyn LlmProvider>, ProviderError>,
) -> Vec<ProviderResolutionCandidate> {
    let win = resolved
        .as_ref()
        .ok()
        .map(|p| success_step(cfg, p.as_ref()));
    let fail = resolved.as_ref().err().map(failure_step);

    vec![
        candidate_bedrock(cfg, win, fail),
        candidate_native_claude(cfg, win, fail),
        candidate_openrouter_slash(cfg, win, fail),
        candidate_azure(cfg, win, fail),
        candidate_openai(cfg, win, fail),
        candidate_groq(cfg, win, fail),
        candidate_openai_compat(cfg, win, fail),
        candidate_ollama_heuristic(cfg, win, fail),
        candidate_ollama_default(cfg, win, fail),
    ]
}

fn candidate_bedrock(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 1u32;
    let gate = bedrock_branch_active(cfg);
    let creds_ok = BedrockBackend::from_env(cfg.aws_region.clone(), cfg.model.clone()).is_some();
    let st = row_state(step, win, fail);

    let (eligible, reason, missing) = if st == "matched" && gate && creds_ok {
        (
            true,
            "Selected: Amazon Bedrock (first resolver branch; SigV4 credentials loaded from the environment).".into(),
            vec![],
        )
    } else if st == "failed_here" {
        (
            false,
            "Resolver failed here: Bedrock branch is active but AWS credentials are incomplete or missing.".into(),
            vec![
                "AWS_ACCESS_KEY_ID".into(),
                "AWS_SECRET_ACCESS_KEY".into(),
            ],
        )
    } else if !gate {
        (
            false,
            "Skipped: Bedrock is considered only when `--bedrock` is set or `AWS_ACCESS_KEY_ID` is present.".into(),
            vec![],
        )
    } else if gate && !creds_ok {
        (
            false,
            "Bedrock branch is active, but credentials could not be loaded (need access key + secret key).".into(),
            vec![
                "AWS_ACCESS_KEY_ID".into(),
                "AWS_SECRET_ACCESS_KEY".into(),
            ],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: resolver continued because Bedrock was not active or did not match.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "bedrock".into(),
        eligible,
        reason,
        missing_prerequisites: missing,
        priority_order: step,
    }
}

fn candidate_native_claude(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 2u32;
    let m = cfg.model.trim();
    let applicable = looks_like_claude_api_model(m) && !m.contains('/');
    let has_anth = nonempty(cfg.anthropic_api_key.clone()).is_some();
    let has_or = nonempty(cfg.openrouter_api_key.clone()).is_some();
    let st = row_state(step, win, fail);

    let (eligible, reason, missing) = if !applicable {
        (
            false,
            "Not applicable: native Claude routing applies only to `claude-*` ids without `/`."
                .into(),
            vec![],
        )
    } else if st == "matched" {
        let id = if has_anth {
            "anthropic"
        } else if has_or {
            "openrouter"
        } else {
            "unknown"
        };
        (
            true,
            format!(
                "Selected: native Claude id routed via `{id}` (Anthropic API if key present, otherwise OpenRouter `anthropic/<model>`)."
            ),
            vec![],
        )
    } else if st == "failed_here" {
        (
            false,
            "Resolver failed here: Claude model id requires `ANTHROPIC_API_KEY` or `OPENROUTER_API_KEY` (or CLI flags).".into(),
            vec!["ANTHROPIC_API_KEY".into(), "OPENROUTER_API_KEY".into()],
        )
    } else if applicable && !has_anth && !has_or && st == "skipped" {
        (
            false,
            "Would fail here without keys: native Claude ids need Anthropic or OpenRouter credentials.".into(),
            vec!["ANTHROPIC_API_KEY".into(), "OPENROUTER_API_KEY".into()],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: Bedrock or another earlier branch applied first.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "native_claude".into(),
        eligible,
        reason,
        missing_prerequisites: missing,
        priority_order: step,
    }
}

fn candidate_openrouter_slash(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 3u32;
    let m = cfg.model.trim();
    let applicable = m.contains('/');
    let has_key = nonempty(cfg.openrouter_api_key.clone()).is_some();
    let st = row_state(step, win, fail);

    let (eligible, reason, missing) = if !applicable {
        (
            false,
            "Not applicable: OpenRouter `org/model` slugs require `/` in the model id.".into(),
            vec![],
        )
    } else if st == "matched" {
        (
            true,
            "Selected: OpenRouter for slash model id.".into(),
            vec![],
        )
    } else if st == "failed_here" {
        (
            false,
            "Resolver failed here: slash model id requires `OPENROUTER_API_KEY` (or `--openrouter-key`).".into(),
            vec!["OPENROUTER_API_KEY".into()],
        )
    } else if applicable && !has_key && st == "skipped" {
        (
            false,
            "Would fail here without key: OpenRouter slash models need an API key.".into(),
            vec!["OPENROUTER_API_KEY".into()],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "openrouter".into(),
        eligible,
        reason,
        missing_prerequisites: missing,
        priority_order: step,
    }
}

fn candidate_azure(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 4u32;
    let ep = nonempty(cfg.azure_openai_endpoint.clone());
    let key = nonempty(cfg.azure_openai_api_key.clone());
    let st = row_state(step, win, fail);

    let (eligible, reason, missing) = if ep.is_some() && key.is_some() && st == "matched" {
        (
            true,
            "Selected: Azure OpenAI (deployment URL + `api-key` header).".into(),
            vec![],
        )
    } else if ep.is_some() && key.is_none() {
        (
            false,
            "Azure endpoint is set but `AZURE_OPENAI_API_KEY` / `--azure-key` is missing (both are required).".into(),
            vec!["AZURE_OPENAI_API_KEY".into()],
        )
    } else if key.is_some() && ep.is_none() {
        (
            false,
            "Azure key is set but `AZURE_OPENAI_ENDPOINT` / `--azure-endpoint` is missing (both are required).".into(),
            vec!["AZURE_OPENAI_ENDPOINT".into()],
        )
    } else if ep.is_none() && key.is_none() {
        (
            false,
            "Not applicable: Azure route needs both endpoint and key.".into(),
            vec![],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "azure_openai".into(),
        eligible,
        reason,
        missing_prerequisites: missing,
        priority_order: step,
    }
}

fn candidate_openai(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 5u32;
    let m = cfg.model.trim();
    let has_key = nonempty(cfg.openai_api_key.clone()).is_some();
    let heur = looks_like_openai_chat_model(m);
    let st = row_state(step, win, fail);

    let (eligible, reason, missing_prerequisites) = if st == "matched" {
        (
            true,
            "Selected: OpenAI cloud API (heuristic `gpt-*` / `o*` chat models).".into(),
            vec![],
        )
    } else if has_key && !heur {
        (
            false,
            "OpenAI API key is present, but the model id does not match the OpenAI chat heuristic; resolver continues.".into(),
            vec![],
        )
    } else if !has_key {
        (
            false,
            "Not applicable: no `OPENAI_API_KEY` / `--openai-key`.".into(),
            vec![],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "openai".into(),
        eligible,
        reason,
        missing_prerequisites,
        priority_order: step,
    }
}

fn candidate_groq(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 6u32;
    let m = cfg.model.trim();
    let has_key = nonempty(cfg.groq_api_key.clone()).is_some();
    let heur = looks_like_groq_hosted_model(m);
    let st = row_state(step, win, fail);

    let (eligible, reason, missing) = if st == "matched" {
        (
            true,
            "Selected: Groq (`llama*` / `mixtral*` heuristic).".into(),
            vec![],
        )
    } else if has_key && !heur {
        (
            false,
            "Groq API key is present, but the model id does not match Groq heuristics; resolver continues.".into(),
            vec![],
        )
    } else if !has_key {
        (
            false,
            "Not applicable: no `GROQ_API_KEY` / `--groq-key`.".into(),
            vec![],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "groq".into(),
        eligible,
        reason,
        missing_prerequisites: missing,
        priority_order: step,
    }
}

fn candidate_openai_compat(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 7u32;
    let url = nonempty(cfg.openai_compatible_url.clone());
    let key = nonempty(cfg.openai_compatible_api_key.clone());
    let st = row_state(step, win, fail);

    let (eligible, reason, missing) = if url.is_some() && key.is_some() && st == "matched" {
        (
            true,
            "Selected: custom OpenAI-compatible server (Bearer auth).".into(),
            vec![],
        )
    } else if st == "failed_here" {
        (
            false,
            "Resolver failed here: `--openai-compatible-url` is set but no API key was provided."
                .into(),
            vec!["OPENAI_COMPATIBLE_API_KEY or --openai-compatible-key".into()],
        )
    } else if url.is_some() && key.is_none() {
        (
            false,
            "OpenAI-compatible base URL is set without a key (required for this branch).".into(),
            vec!["OPENAI_COMPATIBLE_API_KEY or --openai-compatible-key".into()],
        )
    } else if url.is_none() {
        (
            false,
            "Not applicable: no `--openai-compatible-url` (or config).".into(),
            vec![],
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
            vec![],
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
            vec![],
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
            vec![],
        )
    };

    ProviderResolutionCandidate {
        provider: "openai_compatible".into(),
        eligible,
        reason,
        missing_prerequisites: missing,
        priority_order: step,
    }
}

fn candidate_ollama_heuristic(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 8u32;
    let heur = looks_like_ollama_model(&cfg.model);
    let st = row_state(step, win, fail);

    let (eligible, reason) = if st == "matched" && heur {
        (
            true,
            "Selected: Ollama via local-model heuristic (`--ollama-url`).".into(),
        )
    } else if !heur {
        (
            false,
            "Not applicable: Ollama heuristic path does not match this model id.".into(),
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
        )
    };

    ProviderResolutionCandidate {
        provider: "ollama".into(),
        eligible,
        reason,
        missing_prerequisites: vec![],
        priority_order: step,
    }
}

fn candidate_ollama_default(
    cfg: &LlmConnectConfig,
    win: Option<u32>,
    fail: Option<u32>,
) -> ProviderResolutionCandidate {
    let step = 9u32;
    let heur = looks_like_ollama_model(&cfg.model);
    let st = row_state(step, win, fail);

    let (eligible, reason) = if heur {
        (
            false,
            "Not applicable: the Ollama heuristic branch (priority 8) would match instead.".into(),
        )
    } else if st == "matched" {
        (
            true,
            "Selected: Ollama default fallback (no earlier branch matched).".into(),
        )
    } else if st == "superseded" {
        (
            false,
            "Not selected: a later resolver branch matched first.".into(),
        )
    } else if st == "skipped" {
        (
            false,
            "Skipped: earlier resolver branches applied first.".into(),
        )
    } else {
        (
            false,
            "Not evaluated: resolver stopped earlier with an error.".into(),
        )
    };

    ProviderResolutionCandidate {
        provider: "ollama".into(),
        eligible,
        reason,
        missing_prerequisites: vec![],
        priority_order: step,
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_provider_id, explain_provider_resolution};
    use crate::LlmConnectConfig;
    use crate::provider_error::ProviderError;

    #[test]
    fn candidate_priority_orders_are_one_through_nine() {
        let cfg = LlmConnectConfig::default();
        let t = explain_provider_resolution(&cfg);
        let orders: Vec<u32> = t.candidates.iter().map(|c| c.priority_order).collect();
        assert_eq!(orders, (1..=9).collect::<Vec<_>>());
    }

    #[test]
    fn json_has_no_raw_secrets() {
        let cfg = LlmConnectConfig {
            model: "gpt-4o".into(),
            openai_api_key: Some("sk-proj-ABSOLUTESECRETVALUE999".into()),
            openrouter_api_key: Some("sk-or-topsecretvalue".into()),
            ..Default::default()
        };
        let t = explain_provider_resolution(&cfg);
        let s = serde_json::to_string(&t).expect("json");
        assert!(!s.contains("ABSOLUTESECRET"));
        assert!(!s.contains("topsecret"));
    }

    #[test]
    fn parity_selected_id_matches_resolve_when_no_aws_env() {
        if std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            return;
        }
        let cfg = LlmConnectConfig {
            model: "llama3.2".into(),
            ..Default::default()
        };
        let resolved = cfg.clone().resolve().expect("ok");
        let trace = explain_provider_resolution(&cfg);
        assert_eq!(
            trace.selected_provider.as_deref(),
            Some(canonical_provider_id(resolved.as_ref()).as_str())
        );
        assert!(trace.resolution_error.is_none());
    }

    #[test]
    fn snapshot_scenarios_representative() {
        if std::env::var("AWS_ACCESS_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            return;
        }
        let claude_only = LlmConnectConfig {
            model: "claude-haiku-4-5-20251001".into(),
            ..Default::default()
        };
        let t = explain_provider_resolution(&claude_only);
        assert_eq!(
            t.resolution_error,
            Some(ProviderError::ClaudeModelsRequireApiKey)
        );
        assert!(t.selected_provider.is_none());

        let slash = LlmConnectConfig {
            model: "anthropic/claude-3.5-haiku".into(),
            ..Default::default()
        };
        let t2 = explain_provider_resolution(&slash);
        assert_eq!(
            t2.resolution_error,
            Some(ProviderError::OpenRouterKeyRequiredForSlashModel)
        );

        let mismatch = LlmConnectConfig {
            model: "llama3.2".into(),
            openai_api_key: Some("sk-openai-xxxxx".into()),
            ..Default::default()
        };
        let t3 = explain_provider_resolution(&mismatch);
        assert_eq!(t3.selected_provider.as_deref(), Some("ollama"));
        let openai_row = t3
            .candidates
            .iter()
            .find(|c| c.provider == "openai")
            .expect("openai row");
        assert!(
            openai_row.reason.contains("heuristic"),
            "{}",
            openai_row.reason
        );
    }
}
