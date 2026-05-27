//! Launch configuration for the interactive TUI.

use std::path::PathBuf;
#[cfg(feature = "semantic-index")]
use std::sync::{Arc, Mutex};

use akmon_config::TerminalTheme;
use akmon_core::ModelCostEstimateRow;
#[cfg(feature = "semantic-index")]
use akmon_index::RepoIndex;
use akmon_models::LlmConnectConfig;
#[cfg(feature = "semantic-index")]
use fastembed::TextEmbedding;
#[cfg(feature = "semantic-index")]
use tokio::sync::RwLock;
use uuid::Uuid;

/// Shared semantic index handle (optional `--index` mode).
#[cfg(feature = "semantic-index")]
pub type SemanticIndexSlot = (Arc<RwLock<Option<RepoIndex>>>, Arc<Mutex<TextEmbedding>>);

/// Inputs required to open the interactive TUI and run the agent (header, tools, provider, audit).
#[derive(Clone)]
pub struct TuiLaunchConfig {
    /// Semver shown in the header (typically `CARGO_PKG_VERSION` from the CLI crate).
    pub version: String,
    /// Repository / sandbox root directory.
    pub project_root: PathBuf,
    /// Model id or tag passed on the command line.
    pub model_name: String,
    /// Short policy label, e.g. `INTERACTIVE` or `AUTO`.
    pub mode_label: String,
    /// Session identifier for the status bar, audit path, and transcript save file.
    pub session_id: Uuid,
    /// Upper bound on agent iterations (mirrors [`akmon_core::AgentConfig::max_iterations`]).
    pub max_iterations: u32,
    /// Whether `--index` was passed (semantic indexing).
    pub index_enabled: bool,
    /// Optional Anthropic API key (same semantics as CLI `--anthropic-key`).
    pub anthropic_key: Option<String>,
    /// OpenRouter API key (`OPENROUTER_API_KEY` / `--openrouter-key`).
    pub openrouter_key: Option<String>,
    /// OpenAI API key.
    pub openai_key: Option<String>,
    /// Groq API key.
    pub groq_key: Option<String>,
    /// Azure OpenAI deployment URL (full `.../chat/completions` path).
    pub azure_endpoint: Option<String>,
    /// Azure API key.
    pub azure_key: Option<String>,
    /// Azure `api-version` query parameter.
    pub azure_api_version: String,
    /// When `true`, prefer Bedrock if AWS credentials are present (`--bedrock`).
    pub bedrock: bool,
    /// AWS region for Bedrock.
    pub aws_region: String,
    /// Custom OpenAI-compatible base URL.
    pub openai_compatible_url: Option<String>,
    /// API key for [`Self::openai_compatible_url`].
    pub openai_compatible_key: Option<String>,
    /// Ollama base URL when not using Anthropic.
    pub ollama_url: String,
    /// `--shell-allow` patterns.
    pub shell_allow: Vec<String>,
    /// `--web-fetch`
    pub web_fetch: bool,
    /// `--yes-web`
    pub yes_web: bool,
    /// `--yes` (auto-approve reads path in policy).
    pub auto_yes: bool,
    /// `--mcp-server` URLs.
    pub mcp_servers: Vec<String>,
    /// Resolved JSONL audit path for this session.
    pub audit_log_path: PathBuf,
    /// Optional `AKMON.md` body.
    pub akmon_md: Option<String>,
    /// Whether `AKMON.md` exists on disk under [`Self::project_root`] (used for welcome hints).
    pub has_akmon_md: bool,
    /// Whether the sandbox root came from a `.git` work tree (see [`akmon_core::Sandbox::has_git_root`]).
    pub sandbox_has_git_root: bool,
    /// Live semantic index slot when `--index` succeeded (`--index` requires the `semantic-index` feature).
    #[cfg(feature = "semantic-index")]
    pub semantic_index: Option<SemanticIndexSlot>,
    /// Always `None` when this crate was built without `semantic-index`.
    #[cfg(not(feature = "semantic-index"))]
    pub semantic_index: Option<()>,
    /// `--auto-commit`: auto `git commit` after each successful file edit/write.
    pub auto_commit: bool,
    /// Model id for `/architect` planning phase (`--planner-model` / config).
    pub planner_model: String,
    /// TUI contrast (`~/.akmon/config.toml` `[display]`).
    pub display_theme: TerminalTheme,
    /// Optional label for the status line / session prefix (`--name` / `/name`).
    pub session_display_name: Option<String>,
    /// When resuming, prefill transcript from a saved `~/.akmon/sessions/*.json` snapshot.
    pub resume_messages: Option<Vec<crate::message::TuiMessage>>,
    /// When true, reopen the existing journal graph for [`Self::session_id`].
    pub journal_resume: bool,
    /// Per-model context/cost hints from `~/.akmon/config.toml` (`[model_estimates]`).
    pub model_estimates: Vec<ModelCostEstimateRow>,
}

impl TuiLaunchConfig {
    /// Use terminal default foreground for transcript body text (readable on light backgrounds).
    #[must_use]
    pub fn light_body_text(&self) -> bool {
        matches!(self.display_theme, TerminalTheme::Light)
    }

    /// Short cloud or local label for the status bar (mirrors [`LlmConnectConfig::resolve`] priority).
    pub fn provider_display_name(&self) -> String {
        akmon_models::provider_display_name(&self.model_name).to_string()
    }

    /// `true` when the active model is routed via OpenRouter (`/` in id and key present).
    pub fn uses_openrouter(&self) -> bool {
        akmon_models::provider_display_name(&self.model_name) == "OpenRouter"
    }

    /// Local inference (Ollama fallback / no billable cloud keys resolved).
    pub fn is_free_local_inference(&self) -> bool {
        self.provider_display_name() == "Ollama"
    }

    /// Builds [`LlmConnectConfig`] for `model` using the same merge rules as the CLI headless path.
    pub fn llm_connect_for_model(&self, model: String) -> LlmConnectConfig {
        LlmConnectConfig {
            model,
            ollama_url: self.ollama_url.clone(),
            anthropic_api_key: self.anthropic_key.clone(),
            openrouter_api_key: self.openrouter_key.clone(),
            openai_api_key: self.openai_key.clone(),
            groq_api_key: self.groq_key.clone(),
            azure_openai_endpoint: self.azure_endpoint.clone(),
            azure_openai_api_key: self.azure_key.clone(),
            azure_api_version: self.azure_api_version.clone(),
            bedrock_explicit: self.bedrock,
            aws_region: self.aws_region.clone(),
            openai_compatible_url: self.openai_compatible_url.clone(),
            openai_compatible_api_key: self.openai_compatible_key.clone(),
        }
    }

    /// Last path segment of [`Self::project_root`], or `"."` if missing.
    pub fn project_display_name(&self) -> String {
        self.project_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(".")
            .to_string()
    }
}

impl std::fmt::Debug for TuiLaunchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiLaunchConfig")
            .field("version", &self.version)
            .field("project_root", &self.project_root)
            .field("model_name", &self.model_name)
            .field("session_id", &self.session_id)
            .field("index_enabled", &self.index_enabled)
            .field("has_akmon_md", &self.has_akmon_md)
            .field("sandbox_has_git_root", &self.sandbox_has_git_root)
            .field("auto_commit", &self.auto_commit)
            .field("planner_model", &self.planner_model)
            .field("display_theme", &self.display_theme)
            .field("session_display_name", &self.session_display_name)
            .finish_non_exhaustive()
    }
}
