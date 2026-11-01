//! User-level configuration at `~/.akmon/config.toml` (models, API keys, MCP servers, policy).
//!
//! The file holds API keys, so every write creates it with mode `0600` on unix.
//! Sections removed in v3.0 (`[slo]`, `[signing]`) are ignored when present.

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
    /// Override first-token deadline (ms) for all LLM completions.
    ///
    /// Applies after provider-specific defaults (e.g. Ollama model heuristics).
    /// Useful for slow local OpenAI-compatible servers prefilling large contexts.
    ///
    /// Example (`~/.akmon/config.toml`):
    /// ```toml
    /// first_token_deadline_ms = 600_000
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
"#
    }
}

/// Masks an API key for display: the first 8 characters (not bytes, so a non-ASCII key never
/// panics on a char boundary) followed by `****`, or just `****` for keys of 8 chars or fewer.
fn mask_api_key(k: &str) -> String {
    if k.chars().count() <= 8 {
        "****".into()
    } else {
        let prefix: String = k.chars().take(8).collect();
        format!("{prefix}****")
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
///
/// When the file contains archived top-level sections ([`LEGACY_IGNORED_SECTIONS`]), ONE stderr
/// warning per process names them (they are ignored, but kept on disk by [`save_config_to`]).
pub fn load_config_from(path: &Path) -> Result<AkmonGlobalConfig, ConfigError> {
    if !path.is_file() {
        return Ok(AkmonGlobalConfig::default());
    }
    let raw = fs::read_to_string(path)?;
    let cfg = toml::from_str(&raw).map_err(|e| ConfigError::Toml(e.to_string()))?;
    if let Some(warning) = legacy_sections_warning(&raw, path) {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| eprintln!("{warning}"));
    }
    Ok(cfg)
}

/// The one-line warning for archived sections in `raw`, or `None` when there are none.
#[must_use]
pub fn legacy_sections_warning(raw: &str, path: &Path) -> Option<String> {
    let found = legacy_sections_in(raw);
    if found.is_empty() {
        return None;
    }
    let names: Vec<String> = found.iter().map(|s| format!("[{s}]")).collect();
    Some(format!(
        "akmon: warning: {} contains section(s) {} archived in v3.0 and now IGNORED ([slo]: the archived `akmon slo` command; [signing]: the archived post-run signing hook — sign bundles with `akmon bundle sign` or `akmon seal`). They are left in the file untouched.",
        path.display(),
        names.join(", ")
    ))
}

/// Top-level config sections that earlier releases understood and v3.0 ignores.
///
/// `[slo]` belonged to the archived `akmon slo` command; `[signing]` configured the archived
/// external signing hook (Decision D-05), superseded by `akmon bundle sign`.
pub const LEGACY_IGNORED_SECTIONS: &[&str] = &["slo", "signing"];

/// Returns the [`LEGACY_IGNORED_SECTIONS`] present in raw config TOML (empty when unparsable).
#[must_use]
pub fn legacy_sections_in(raw: &str) -> Vec<&'static str> {
    let Ok(table) = raw.parse::<toml::Table>() else {
        return Vec::new();
    };
    LEGACY_IGNORED_SECTIONS
        .iter()
        .copied()
        .filter(|k| table.contains_key(*k))
        .collect()
}

/// Writes config atomically to `path` (parent dirs must exist).
///
/// The file can hold API keys: on unix the temporary file is created with mode `0600` (and
/// forced to `0600` if a stale temporary file already existed) before it is renamed over
/// `path`, so a write never widens permissions.
///
/// Top-level keys of the existing file that this version does not understand (for example the
/// archived `[slo]` / `[signing]` sections, or settings from a newer release) are preserved: they
/// are merged back into the written file instead of being silently dropped.
pub fn save_config_to(path: &Path, cfg: &AkmonGlobalConfig) -> Result<(), ConfigError> {
    let mut table = toml::Table::try_from(cfg).map_err(|e| ConfigError::Toml(e.to_string()))?;
    if let Ok(existing) = fs::read_to_string(path) {
        for (key, value) in unknown_top_level_entries(&existing) {
            table.entry(key).or_insert(value);
        }
    }
    let raw = toml::to_string_pretty(&table).map_err(|e| ConfigError::Toml(e.to_string()))?;
    let tmp = path.with_extension("toml.tmp");
    write_private_file(&tmp, raw.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Top-level entries of `raw` that [`AkmonGlobalConfig`] does not model.
///
/// Computed as the keys of `raw` that do not survive a parse/re-serialize round trip through
/// [`AkmonGlobalConfig`]. A known field written with its default value can also land here; it is
/// only re-inserted when the new config omits that key, which carries the same meaning. Returns
/// nothing when `raw` is not valid TOML or not a valid config.
fn unknown_top_level_entries(raw: &str) -> Vec<(String, toml::Value)> {
    let Ok(existing) = raw.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Ok(parsed) = toml::from_str::<AkmonGlobalConfig>(raw) else {
        return Vec::new();
    };
    let Ok(known) = toml::Table::try_from(&parsed) else {
        return Vec::new();
    };
    existing
        .into_iter()
        .filter(|(key, _)| !known.contains_key(key))
        .collect()
}

/// Writes `bytes` to `path`, truncating it, with owner-only permissions on unix.
fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(bytes)?;
    f.sync_all()?;
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
    fn first_token_deadline_ms_roundtrip_in_toml() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let c = AkmonGlobalConfig {
            first_token_deadline_ms: Some(600_000),
            ..Default::default()
        };
        save_config_to(&path, &c).expect("save");
        let l = load_config_from(&path).expect("load");
        assert_eq!(l.first_token_deadline_ms, Some(600_000));
    }

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
    fn legacy_slo_and_signing_sections_are_ignored_not_errors() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let raw = r#"default_model = "llama3.2"

[signing]
command = ["/usr/local/bin/akmon-sign.sh"]
timeout_secs = 60

[slo]
min_tool_success_rate = 0.95

[slo.trend]
min_baseline_samples = 5
"#;
        fs::write(&path, raw).expect("write");
        let l = load_config_from(&path).expect("legacy sections must not fail parsing");
        assert_eq!(l.default_model.as_deref(), Some("llama3.2"));
        assert_eq!(legacy_sections_in(raw), vec!["slo", "signing"]);
        assert!(legacy_sections_in("default_model = \"x\"\n").is_empty());
        assert!(legacy_sections_in("not [valid toml").is_empty());
    }

    #[test]
    fn save_preserves_legacy_and_unknown_top_level_sections() {
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let raw = r#"default_model = "llama3.2"
anthropic_api_key = "sk-old"
future_flag = true

[signing]
command = ["/usr/local/bin/akmon-sign.sh"]

[slo]
min_tool_success_rate = 0.95
"#;
        fs::write(&path, raw).expect("write");
        let warning = legacy_sections_warning(raw, &path).expect("warning");
        assert!(warning.contains("[slo], [signing]"), "{warning}");
        assert!(legacy_sections_warning("default_model = \"x\"\n", &path).is_none());

        let mut cfg = load_config_from(&path).expect("load");
        cfg.default_model = Some("qwen".into());
        cfg.anthropic_api_key = None; // a known key the user cleared must stay cleared
        save_config_to(&path, &cfg).expect("save");

        let written = fs::read_to_string(&path).expect("read");
        let table: toml::Table = written.parse().expect("toml");
        assert_eq!(
            table.get("default_model").and_then(toml::Value::as_str),
            Some("qwen")
        );
        assert!(table.get("anthropic_api_key").is_none(), "{written}");
        assert_eq!(
            table.get("future_flag").and_then(toml::Value::as_bool),
            Some(true)
        );
        assert!(
            table.get("signing").is_some_and(toml::Value::is_table),
            "{written}"
        );
        assert!(
            table.get("slo").is_some_and(toml::Value::is_table),
            "{written}"
        );
        assert_eq!(legacy_sections_in(&written), vec!["slo", "signing"]);
    }

    #[test]
    fn mask_api_key_is_char_boundary_safe() {
        assert_eq!(mask_api_key("short"), "****");
        assert_eq!(mask_api_key("ключ-ключ-ключ"), "ключ-клю****");
        assert_eq!(mask_api_key("ééééééééé"), "éééééééé****");
        assert_eq!(mask_api_key("sk-ant-api03-x"), "sk-ant-a****");
    }

    #[cfg(unix)]
    #[test]
    fn save_config_writes_owner_only_and_tightens_existing_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        // Pre-existing world-readable config and a stale, world-writable tmp file.
        fs::write(&path, "").expect("seed");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("perm");
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, "stale").expect("stale tmp");
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o666)).expect("perm tmp");
        let c = AkmonGlobalConfig {
            anthropic_api_key: Some("sk-ant-secret".into()),
            ..Default::default()
        };
        save_config_to(&path, &c).expect("save");
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "config with API keys must be 0600, got {mode:o}"
        );
        let l = load_config_from(&path).expect("load");
        assert_eq!(l.anthropic_api_key.as_deref(), Some("sk-ant-secret"));
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
