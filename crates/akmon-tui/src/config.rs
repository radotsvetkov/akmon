//! Launch configuration for the interactive TUI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use akmon_index::RepoIndex;
use fastembed::TextEmbedding;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Shared semantic index handle (optional `--index` mode).
pub type SemanticIndexSlot = (
    Arc<RwLock<Option<RepoIndex>>>,
    Arc<Mutex<TextEmbedding>>,
);

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
    /// Live semantic index slot when `--index` succeeded.
    pub semantic_index: Option<SemanticIndexSlot>,
    /// `--auto-commit`: auto `git commit` after each successful file edit/write.
    pub auto_commit: bool,
}

impl TuiLaunchConfig {
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
            .finish_non_exhaustive()
    }
}
