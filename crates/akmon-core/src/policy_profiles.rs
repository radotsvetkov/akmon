//! Built-in policy profiles and deterministic merge/load helpers.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::policy::PolicyConfig;

/// Built-in enterprise policy profile name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyProfileName {
    /// Developer-friendly profile with controlled writes/shell/network.
    Dev,
    /// Staging profile with stricter side effects than `dev`.
    Staging,
    /// Production profile with highly restrictive explicit-deny posture.
    Prod,
}

/// Policy pack loading/validation errors.
#[derive(Debug, Error)]
pub enum PolicyPackError {
    /// Pack file I/O failure.
    #[error("policy pack I/O error for `{path}`: {message}")]
    Io {
        /// Pack path.
        path: String,
        /// Detail.
        message: String,
    },
    /// Pack format parse failure.
    #[error("policy pack parse error for `{path}`: {message}")]
    Parse {
        /// Pack path.
        path: String,
        /// Detail.
        message: String,
    },
}

/// Returns the built-in policy defaults for `profile`.
pub fn built_in_policy_profile(profile: PolicyProfileName) -> PolicyConfig {
    match profile {
        PolicyProfileName::Dev => {
            let mut cfg = PolicyConfig::default();
            cfg.filesystem.read.allow = vec!["**".into()];
            cfg.filesystem.read.deny = vec![".git/**".into(), "**/.env*".into()];
            cfg.filesystem.write.allow = vec![
                "src/**".into(),
                "tests/**".into(),
                "docs/**".into(),
                "README.md".into(),
                "Cargo.toml".into(),
                "Cargo.lock".into(),
                ".akmon/specs/**".into(),
                ".akmon/plans/**".into(),
            ];
            cfg.filesystem.write.deny = vec![
                ".git/**".into(),
                "**/*.pem".into(),
                "**/*.key".into(),
                "**/.env*".into(),
            ];
            cfg.shell.allow_prefixes = vec![
                "cargo ".into(),
                "rustfmt ".into(),
                "git status".into(),
                "git diff".into(),
                "git log".into(),
                "npm test".into(),
                "pnpm test".into(),
                "pytest ".into(),
            ];
            cfg.shell.deny_prefixes = vec![
                "rm -rf ".into(),
                "git push --force".into(),
                "git push -f".into(),
                "sudo ".into(),
            ];
            cfg.network.allow_domains = vec![
                "api.github.com".into(),
                "raw.githubusercontent.com".into(),
                "docs.rs".into(),
                "crates.io".into(),
            ];
            cfg.network.deny_domains = vec![
                "169.254.169.254".into(),
                "*.internal".into(),
                "localhost".into(),
                "127.0.0.1".into(),
            ];
            cfg.tools.allow = vec![
                "read_*".into(),
                "list_directory".into(),
                "search".into(),
                "write_file".into(),
                "edit".into(),
                "patch".into(),
                "apply_patch".into(),
                "shell".into(),
                "web_fetch".into(),
                "git".into(),
                "read_spec".into(),
                "write_spec".into(),
                "ask_followup".into(),
                "todo_write".into(),
                "spawn_subagent".into(),
            ];
            cfg
        }
        PolicyProfileName::Staging => {
            let mut cfg = PolicyConfig::default();
            cfg.filesystem.read.allow = vec![
                "src/**".into(),
                "tests/**".into(),
                "docs/**".into(),
                "Cargo.toml".into(),
                "Cargo.lock".into(),
                "README.md".into(),
                ".akmon/specs/**".into(),
            ];
            cfg.filesystem.read.deny = vec![".git/**".into(), "**/.env*".into()];
            cfg.filesystem.write.allow = vec![
                "src/**".into(),
                "tests/**".into(),
                "docs/**".into(),
                ".akmon/specs/**".into(),
                ".akmon/plans/**".into(),
            ];
            cfg.filesystem.write.deny = vec![
                ".git/**".into(),
                "**/*.pem".into(),
                "**/*.key".into(),
                "**/.env*".into(),
            ];
            cfg.shell.allow_prefixes = vec![
                "cargo test".into(),
                "cargo fmt".into(),
                "cargo clippy".into(),
                "git status".into(),
                "git diff".into(),
            ];
            cfg.shell.deny_prefixes = vec![
                "rm -rf ".into(),
                "git commit".into(),
                "git push".into(),
                "sudo ".into(),
            ];
            cfg.network.allow_domains = vec!["api.github.com".into(), "docs.rs".into()];
            cfg.network.deny_domains = vec![
                "169.254.169.254".into(),
                "*.internal".into(),
                "localhost".into(),
                "127.0.0.1".into(),
            ];
            cfg.tools.allow = vec![
                "read_*".into(),
                "list_directory".into(),
                "search".into(),
                "write_file".into(),
                "edit".into(),
                "patch".into(),
                "apply_patch".into(),
                "read_spec".into(),
                "write_spec".into(),
                "ask_followup".into(),
                "todo_write".into(),
            ];
            cfg.tools.deny = vec!["web_fetch".into(), "git".into(), "spawn_subagent".into()];
            cfg
        }
        PolicyProfileName::Prod => {
            let mut cfg = PolicyConfig::default();
            cfg.filesystem.read.allow = vec![
                "src/**".into(),
                "docs/**".into(),
                "Cargo.toml".into(),
                "Cargo.lock".into(),
                "README.md".into(),
                ".akmon/specs/**".into(),
            ];
            cfg.filesystem.read.deny = vec![".git/**".into(), "**/.env*".into()];
            cfg.filesystem.write.allow = Vec::new();
            cfg.filesystem.write.deny = vec!["**".into()];
            cfg.shell.allow_prefixes = Vec::new();
            cfg.shell.deny_prefixes = Vec::new();
            cfg.network.allow_domains = Vec::new();
            cfg.network.deny_domains = vec!["**".into()];
            cfg.tools.allow = vec![
                "read_*".into(),
                "list_directory".into(),
                "search".into(),
                "read_spec".into(),
                "ask_followup".into(),
                "todo_write".into(),
            ];
            cfg.tools.deny = vec![
                "write_*".into(),
                "edit".into(),
                "patch".into(),
                "apply_patch".into(),
                "shell".into(),
                "web_fetch".into(),
                "git".into(),
                "spawn_subagent".into(),
                "memory_write".into(),
            ];
            cfg
        }
    }
}

/// Deterministically merges `overlay` on top of `base`.
///
/// Merge semantics:
/// - list fields are appended (`base` then `overlay`);
/// - duplicates are removed while keeping the **last** occurrence, so later
///   precedence layers remain later in declaration order for tie-breaking.
pub fn merge_policy_config(base: &PolicyConfig, overlay: &PolicyConfig) -> PolicyConfig {
    let mut merged = base.clone();
    merged.filesystem.read.allow =
        merge_rule_list(&base.filesystem.read.allow, &overlay.filesystem.read.allow);
    merged.filesystem.read.deny =
        merge_rule_list(&base.filesystem.read.deny, &overlay.filesystem.read.deny);
    merged.filesystem.write.allow = merge_rule_list(
        &base.filesystem.write.allow,
        &overlay.filesystem.write.allow,
    );
    merged.filesystem.write.deny =
        merge_rule_list(&base.filesystem.write.deny, &overlay.filesystem.write.deny);
    merged.shell.allow_prefixes =
        merge_rule_list(&base.shell.allow_prefixes, &overlay.shell.allow_prefixes);
    merged.shell.deny_prefixes =
        merge_rule_list(&base.shell.deny_prefixes, &overlay.shell.deny_prefixes);
    merged.network.allow_domains =
        merge_rule_list(&base.network.allow_domains, &overlay.network.allow_domains);
    merged.network.deny_domains =
        merge_rule_list(&base.network.deny_domains, &overlay.network.deny_domains);
    merged.tools.allow = merge_rule_list(&base.tools.allow, &overlay.tools.allow);
    merged.tools.deny = merge_rule_list(&base.tools.deny, &overlay.tools.deny);
    merged.mcp.servers.allow = merge_rule_list(&base.mcp.servers.allow, &overlay.mcp.servers.allow);
    merged.mcp.servers.deny = merge_rule_list(&base.mcp.servers.deny, &overlay.mcp.servers.deny);
    merged.mcp.tools.allow = merge_rule_list(&base.mcp.tools.allow, &overlay.mcp.tools.allow);
    merged.mcp.tools.deny = merge_rule_list(&base.mcp.tools.deny, &overlay.mcp.tools.deny);
    merged
}

fn merge_rule_list(base: &[String], overlay: &[String]) -> Vec<String> {
    let mut all = Vec::with_capacity(base.len().saturating_add(overlay.len()));
    all.extend(base.iter().cloned());
    all.extend(overlay.iter().cloned());
    dedup_keep_last(all)
}

fn dedup_keep_last(values: Vec<String>) -> Vec<String> {
    let mut out_rev: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for v in values.into_iter().rev() {
        if seen.insert(v.clone()) {
            out_rev.push(v);
        }
    }
    out_rev.into_iter().rev().collect()
}

/// Parses one policy config/pack file (`json` or `toml`).
pub fn parse_policy_config_file(path: &Path) -> Result<PolicyConfig, PolicyPackError> {
    let raw = std::fs::read_to_string(path).map_err(|e| PolicyPackError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    match ext {
        "json" => serde_json::from_str::<PolicyConfig>(&raw).map_err(|e| PolicyPackError::Parse {
            path: path.display().to_string(),
            message: e.to_string(),
        }),
        "toml" => toml::from_str::<PolicyConfig>(&raw).map_err(|e| PolicyPackError::Parse {
            path: path.display().to_string(),
            message: e.to_string(),
        }),
        _ => {
            if let Ok(v) = serde_json::from_str::<PolicyConfig>(&raw) {
                return Ok(v);
            }
            toml::from_str::<PolicyConfig>(&raw).map_err(|e| PolicyPackError::Parse {
                path: path.display().to_string(),
                message: e.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_profiles_have_expected_shape() {
        let dev = built_in_policy_profile(PolicyProfileName::Dev);
        let staging = built_in_policy_profile(PolicyProfileName::Staging);
        let prod = built_in_policy_profile(PolicyProfileName::Prod);
        assert!(!dev.filesystem.write.allow.is_empty());
        assert!(!staging.shell.allow_prefixes.is_empty());
        assert!(prod.filesystem.write.allow.is_empty());
        assert!(!prod.filesystem.write.deny.is_empty());
    }

    #[test]
    fn merge_preserves_overlay_precedence_in_list_order() {
        let mut base = PolicyConfig::default();
        base.tools.allow = vec!["read_*".into(), "search".into()];
        base.mcp.tools.allow = vec!["search_*".into()];
        let mut overlay = PolicyConfig::default();
        overlay.tools.allow = vec!["search".into(), "write_file".into()];
        overlay.mcp.tools.allow = vec!["search_*".into(), "list_*".into()];
        let merged = merge_policy_config(&base, &overlay);
        assert_eq!(
            merged.tools.allow,
            vec![
                "read_*".to_string(),
                "search".to_string(),
                "write_file".to_string()
            ]
        );
        assert_eq!(
            merged.mcp.tools.allow,
            vec!["search_*".to_string(), "list_*".to_string()]
        );
    }

    #[test]
    fn parse_policy_config_file_rejects_invalid_pack() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "[policy\ninvalid").expect("write");
        let err = parse_policy_config_file(&path).expect_err("invalid");
        assert!(err.to_string().contains("parse error"));
    }
}
