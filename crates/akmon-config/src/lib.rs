//! User-level configuration at `~/.akmon/config.toml` (models, API keys, MCP servers, SLO defaults).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Where MCP entries apply: user-wide vs project-specific file (future project path).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    /// Stored in `~/.akmon/config.toml`.
    #[default]
    User,
    /// Intended for project-local config (same file section for now).
    Project,
}

/// One HTTP MCP server entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Short name (e.g. `github`).
    pub name: String,
    /// Base URL for the MCP HTTP endpoint.
    pub url: String,
    /// When `false`, the CLI should skip registering this server.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub scope: McpScope,
}

fn default_true() -> bool {
    true
}

/// Defaults for `--architect` / `/architect` (planning model).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectConfig {
    /// Ollama tag or Claude id used only for the planning phase.
    #[serde(default)]
    pub planner_model: Option<String>,
}

/// Terminal background / contrast hint for the interactive UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalTheme {
    /// Assume a dark background (Akmon’s default palette).
    #[default]
    Auto,
    /// Force dark-theme colors.
    Dark,
    /// Prefer high-contrast readable text on light backgrounds.
    Light,
}

/// Display-related options in `~/.akmon/config.toml` (`[display]`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// `auto` keeps the dark palette unless set to `light`.
    #[serde(default)]
    pub theme: TerminalTheme,
}

/// SLO defaults under `[slo]` and `[slo.trend]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SloConfig {
    /// Single-run threshold checks (`akmon slo verify`).
    #[serde(flatten)]
    pub thresholds: akmon_core::ReliabilitySloThresholds,
    /// Trend/regression guardrail checks (`akmon slo trend`).
    #[serde(default)]
    pub trend: akmon_core::RegressionGuardConfig,
}

/// Policy profile/pack defaults under `[policy]`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyGovernanceConfig {
    /// Default built-in profile (`dev`, `staging`, `prod`).
    pub profile: Option<akmon_core::PolicyProfileName>,
    /// Additional policy pack paths loaded in listed order.
    pub packs: Vec<String>,
}

/// Serializable contents of `~/.akmon/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AkmonGlobalConfig {
    /// Default model id (Ollama tag or Claude id).
    #[serde(default)]
    pub default_model: Option<String>,
    /// Ollama API base URL.
    #[serde(default)]
    pub ollama_url: Option<String>,
    /// Stored Anthropic API key (treat as secret on disk).
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    /// OpenRouter (`https://openrouter.ai`).
    #[serde(default)]
    pub openrouter_api_key: Option<String>,
    /// OpenAI (`api.openai.com`).
    #[serde(default)]
    pub openai_api_key: Option<String>,
    /// Groq.
    #[serde(default)]
    pub groq_api_key: Option<String>,
    /// Azure OpenAI HTTPS endpoint ending in `/deployments/.../chat/completions`.
    #[serde(default)]
    pub azure_openai_endpoint: Option<String>,
    #[serde(default)]
    pub azure_openai_api_key: Option<String>,
    #[serde(default)]
    pub azure_api_version: Option<String>,
    /// Any OpenAI-compatible server base (no `/chat/completions` suffix).
    #[serde(default)]
    pub openai_compatible_url: Option<String>,
    #[serde(default)]
    pub openai_compatible_api_key: Option<String>,
    /// Override for the first-token deadline (ms) applied to all LLM completions.
    ///
    /// Useful for local models (lemonade, mlx-lm, Ollama) that must prefill large
    /// contexts before emitting a single token.  When unset, [`CompletionConfig`]
    /// default (currently 300 000 ms / 5 min) is used.
    ///
    /// Example (`~/.akmon/config.toml`):
    /// ```toml
    /// first_token_deadline_ms = 600_000  # 10 min for very large contexts
    /// ```
    #[serde(default)]
    pub first_token_deadline_ms: Option<u64>,
    /// Registered MCP HTTP servers.
    #[serde(default)]
    pub mcp: Vec<McpServerEntry>,
    /// Architect / two-phase planning defaults.
    #[serde(default)]
    pub architect: ArchitectConfig,
    /// TUI typography / contrast (`[display]`).
    #[serde(default)]
    pub display: DisplayConfig,
    /// Optional per-model context-window and USD hints for status bars, `/context`, and headless budget math.
    #[serde(default)]
    pub model_estimates: Vec<akmon_core::ModelCostEstimateRow>,
    /// Default reliability SLO settings (`[slo]`) for verify + trend checks.
    #[serde(default)]
    pub slo: SloConfig,
    /// Enterprise policy profile/pack defaults.
    #[serde(default)]
    pub policy: PolicyGovernanceConfig,
}

impl AkmonGlobalConfig {
    /// Masks secret values for display (`sk-ant-…` → `sk-ant-****`).
    pub fn display_masked_toml(&self) -> String {
        let mut c = self.clone();
        if let Some(ref k) = c.anthropic_api_key {
            c.anthropic_api_key = Some(mask_api_key(k));
        }
        if let Some(ref k) = c.openrouter_api_key {
            c.openrouter_api_key = Some(mask_api_key(k));
        }
        if let Some(ref k) = c.openai_api_key {
            c.openai_api_key = Some(mask_api_key(k));
        }
        if let Some(ref k) = c.groq_api_key {
            c.groq_api_key = Some(mask_api_key(k));
        }
        if let Some(ref k) = c.azure_openai_api_key {
            c.azure_openai_api_key = Some(mask_api_key(k));
        }
        if let Some(ref k) = c.openai_compatible_api_key {
            c.openai_compatible_api_key = Some(mask_api_key(k));
        }
        toml::to_string_pretty(&c).unwrap_or_else(|_| "# (invalid config)\n".into())
    }

    /// Commented starter TOML documenting optional provider keys (for docs / wizard output).
    pub fn annotated_template() -> &'static str {
        r#"# Akmon user config (~/.akmon/config.toml)
# default_model = "llama3.2"
# ollama_url = "http://localhost:11434"

# anthropic_api_key = "sk-ant-..."
# openrouter_api_key = "sk-or-..."
# openai_api_key = "sk-..."
# groq_api_key = "gsk_..."

# Azure: full deployment URL + api-version is appended if missing from the URL
# azure_openai_endpoint = "https://MYRESOURCE.openai.azure.com/openai/deployments/MYDEPLOYMENT/chat/completions"
# azure_openai_api_key = "..."
# azure_api_version = "2024-02-01"

# LM Studio / local OpenAI-compatible
# openai_compatible_url = "http://127.0.0.1:1234/v1"
# openai_compatible_api_key = "lm-studio"  # if required

# Amazon Bedrock: use CLI --bedrock and AWS_* env vars (see README)

# [architect]
# planner_model = "llama3.2"

# [policy]
# profile = "dev"
# packs = [".akmon/policy-packs/team.toml"]

# [slo]
# min_tool_success_rate = 0.95
# max_timeout_rate = 0.02
# max_tool_failure_rate = 0.05
# max_retries_total = 3
# max_timeouts_total = 2
# min_tool_calls_total = 5
#
# [slo.trend]
# max_success_rate_drop_abs = 0.05
# max_timeout_rate_increase_abs = 0.02
# max_failure_rate_increase_abs = 0.03
# max_retries_increase_ratio = 1.0
# max_latency_avg_increase_ratio = 0.50
# min_baseline_samples = 5
"#
    }
}

fn mask_api_key(k: &str) -> String {
    if k.len() <= 8 {
        "****".into()
    } else {
        format!("{}****", &k[..8])
    }
}

/// Errors loading or writing global config.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// I/O failure.
    #[error("config I/O: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse/serialize error.
    #[error("config TOML: {0}")]
    Toml(String),
}

/// Returns `~/.akmon` (creating directories is caller responsibility for parent chain).
pub fn akmon_config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".akmon"))
}

/// Path to `~/.akmon/config.toml` when home is known.
pub fn akmon_config_path() -> Option<PathBuf> {
    akmon_config_dir().map(|d| d.join("config.toml"))
}

/// Reads config from `path`, or returns defaults if missing.
pub fn load_config_from(path: &Path) -> Result<AkmonGlobalConfig, ConfigError> {
    if !path.is_file() {
        return Ok(AkmonGlobalConfig::default());
    }
    let raw = fs::read_to_string(path)?;
    toml::from_str(&raw).map_err(|e| ConfigError::Toml(e.to_string()))
}

/// Writes config atomically to `path` (parent dirs must exist).
pub fn save_config_to(path: &Path, cfg: &AkmonGlobalConfig) -> Result<(), ConfigError> {
    let raw = toml::to_string_pretty(cfg).map_err(|e| ConfigError::Toml(e.to_string()))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Ensures `~/.akmon` exists and loads `config.toml`.
pub fn load_user_config() -> Result<(PathBuf, AkmonGlobalConfig), ConfigError> {
    let Some(dir) = akmon_config_dir() else {
        return Err(ConfigError::Toml("no home directory".into()));
    };
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let cfg = load_config_from(&path)?;
    Ok((path, cfg))
}

/// Saves to `~/.akmon/config.toml`.
pub fn save_user_config(cfg: &AkmonGlobalConfig) -> Result<PathBuf, ConfigError> {
    let (path, _) = load_user_config()?;
    save_config_to(&path, cfg)?;
    Ok(path)
}

/// Appends `.akmon/` to `.gitignore` in `cwd` if the file exists and does not already mention it.
pub fn append_akmon_gitignore_line(cwd: &Path) -> std::io::Result<bool> {
    let gi = cwd.join(".gitignore");
    if !gi.is_file() {
        return Ok(false);
    }
    let existing = fs::read_to_string(&gi)?;
    if existing
        .lines()
        .any(|l| l.trim() == ".akmon/" || l.trim() == ".akmon")
    {
        return Ok(false);
    }
    let mut out = existing;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n# Added by akmon config key set\n.akmon/\n");
    fs::write(&gi, out)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn model_roundtrip_save() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let c = AkmonGlobalConfig {
            default_model: Some("llama3.2".into()),
            ..Default::default()
        };
        save_config_to(&path, &c).expect("save");
        let l = load_config_from(&path).expect("load");
        assert_eq!(l.default_model.as_deref(), Some("llama3.2"));
    }

    #[test]
    fn mcp_add_and_remove() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let mut c = AkmonGlobalConfig::default();
        c.mcp.push(McpServerEntry {
            name: "github".into(),
            url: "https://mcp.example.com".into(),
            enabled: true,
            scope: McpScope::User,
        });
        save_config_to(&path, &c).expect("save");
        let mut l = load_config_from(&path).expect("load");
        assert_eq!(l.mcp.len(), 1);
        l.mcp.retain(|e| e.name != "github");
        save_config_to(&path, &l).expect("save");
        let l2 = load_config_from(&path).expect("load");
        assert!(l2.mcp.is_empty());
    }

    #[test]
    fn mcp_disable_sets_enabled_false() {
        let mut c = AkmonGlobalConfig::default();
        c.mcp.push(McpServerEntry {
            name: "x".into(),
            url: "http://x".into(),
            enabled: true,
            scope: McpScope::User,
        });
        if let Some(e) = c.mcp.iter_mut().find(|e| e.name == "x") {
            e.enabled = false;
        }
        assert!(!c.mcp[0].enabled);
    }

    #[test]
    fn show_masks_api_key() {
        let c = AkmonGlobalConfig {
            anthropic_api_key: Some("sk-ant-api03-abcdef123456".into()),
            ..Default::default()
        };
        let s = c.display_masked_toml();
        assert!(!s.contains("abcdef123456"));
        assert!(s.contains("sk-ant-a"));
    }

    #[test]
    fn model_estimates_roundtrip_in_toml() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let mut c = AkmonGlobalConfig::default();
        c.model_estimates.push(akmon_core::ModelCostEstimateRow {
            pattern: "claude-haiku".into(),
            context_window_tokens: Some(200_000),
            input_per_million_usd: Some(0.9),
            output_per_million_usd: None,
            cache_read_per_million_usd: None,
            note: Some("Context % is not a rate limit meter.".into()),
        });
        save_config_to(&path, &c).expect("save");
        let l = load_config_from(&path).expect("load");
        assert_eq!(l.model_estimates.len(), 1);
        assert_eq!(l.model_estimates[0].pattern, "claude-haiku");
        assert_eq!(l.model_estimates[0].context_window_tokens, Some(200_000));
    }

    #[test]
    fn slo_thresholds_roundtrip_in_toml() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let c = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: akmon_core::ReliabilitySloThresholds {
                    min_tool_success_rate: Some(0.95),
                    max_timeouts_total: Some(2),
                    ..Default::default()
                },
                trend: akmon_core::RegressionGuardConfig {
                    max_success_rate_drop_abs: Some(0.1),
                    min_baseline_samples: Some(5),
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        save_config_to(&path, &c).expect("save");
        let l = load_config_from(&path).expect("load");
        assert_eq!(l.slo.thresholds.min_tool_success_rate, Some(0.95));
        assert_eq!(l.slo.thresholds.max_timeouts_total, Some(2));
        assert_eq!(l.slo.trend.max_success_rate_drop_abs, Some(0.1));
    }

    #[test]
    fn policy_governance_roundtrip_in_toml() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let c = AkmonGlobalConfig {
            policy: PolicyGovernanceConfig {
                profile: Some(akmon_core::PolicyProfileName::Staging),
                packs: vec![".akmon/policy-packs/org.toml".into()],
            },
            ..Default::default()
        };
        save_config_to(&path, &c).expect("save");
        let l = load_config_from(&path).expect("load");
        assert_eq!(
            l.policy.profile,
            Some(akmon_core::PolicyProfileName::Staging)
        );
        assert_eq!(l.policy.packs.len(), 1);
    }
}
