//! Policy engine: deny-all default, interactive resolution, and configured rules.

use chrono::Utc;
use glob::Pattern;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::audit::{AuditEvent, PolicyVerdict};
use crate::permission::Permission;

/// Rule set loaded from project configuration.
///
/// The evaluator is deterministic and follows two global rules:
///
/// - explicit deny wins over allow;
/// - when multiple rules in the same allow/deny set match, the most specific
///   rule wins (ties resolve by declaration order).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    /// Filesystem path rules for read and write capabilities.
    pub filesystem: FilesystemPolicyConfig,
    /// Shell command prefix rules.
    pub shell: ShellPolicyConfig,
    /// Network domain rules (host portion of URL).
    pub network: NetworkPolicyConfig,
    /// Tool-name rules for dispatch-time checks.
    pub tools: ToolPolicyConfig,
}

/// Read/write filesystem policy buckets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilesystemPolicyConfig {
    /// Rules for [`Permission::ReadFile`] and [`Permission::ListDirectory`].
    pub read: PatternRuleSet,
    /// Rules for [`Permission::WriteFile`].
    pub write: PatternRuleSet,
}

/// Glob-style allow/deny rule set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternRuleSet {
    /// Allow patterns.
    pub allow: Vec<String>,
    /// Deny patterns.
    pub deny: Vec<String>,
}

/// Prefix rules for shell commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellPolicyConfig {
    /// Command prefixes that are permitted.
    pub allow_prefixes: Vec<String>,
    /// Command prefixes that are blocked.
    pub deny_prefixes: Vec<String>,
}

/// Domain rules for network fetches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkPolicyConfig {
    /// Host/domain glob patterns that are permitted.
    pub allow_domains: Vec<String>,
    /// Host/domain glob patterns that are blocked.
    pub deny_domains: Vec<String>,
}

/// Tool dispatch rules keyed by tool name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPolicyConfig {
    /// Tool-name allow patterns.
    pub allow: Vec<String>,
    /// Tool-name deny patterns.
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuleMatch {
    pattern: String,
    specificity: usize,
    declaration_index: usize,
}

impl PolicyConfig {
    /// Applies configured rules to `permission`.
    pub fn evaluate_permission(&self, permission: &Permission) -> (PolicyVerdict, String) {
        match permission {
            Permission::ReadFile { path } | Permission::ListDirectory { path } => {
                let subject = normalize_path_for_policy(path);
                evaluate_pattern_rules(
                    "filesystem.read",
                    "path",
                    &subject,
                    &self.filesystem.read.allow,
                    &self.filesystem.read.deny,
                )
            }
            Permission::WriteFile { path } => {
                let subject = normalize_path_for_policy(path);
                evaluate_pattern_rules(
                    "filesystem.write",
                    "path",
                    &subject,
                    &self.filesystem.write.allow,
                    &self.filesystem.write.deny,
                )
            }
            Permission::ExecuteCommand { command, .. } => evaluate_prefix_rules(
                "shell",
                "command",
                command,
                &self.shell.allow_prefixes,
                &self.shell.deny_prefixes,
            ),
            Permission::NetworkFetch { url } => {
                let Some(domain) = extract_domain(url) else {
                    return (
                        PolicyVerdict::Deny,
                        format!("denied: network URL has no valid domain: `{url}`"),
                    );
                };
                evaluate_pattern_rules(
                    "network",
                    "domain",
                    &domain,
                    &self.network.allow_domains,
                    &self.network.deny_domains,
                )
            }
        }
    }

    /// Applies configured rules to a `tool_name`.
    pub fn evaluate_tool_name(&self, tool_name: &str) -> (PolicyVerdict, String) {
        evaluate_pattern_rules(
            "tool",
            "name",
            tool_name,
            &self.tools.allow,
            &self.tools.deny,
        )
    }

    fn has_tool_rules(&self) -> bool {
        !self.tools.allow.is_empty() || !self.tools.deny.is_empty()
    }
}

fn evaluate_pattern_rules(
    scope: &str,
    subject_kind: &str,
    subject: &str,
    allow: &[String],
    deny: &[String],
) -> (PolicyVerdict, String) {
    let deny_match = best_pattern_match(deny, subject);
    let allow_match = best_pattern_match(allow, subject);
    evaluate_matches(scope, subject_kind, subject, allow_match, deny_match)
}

fn evaluate_prefix_rules(
    scope: &str,
    subject_kind: &str,
    subject: &str,
    allow: &[String],
    deny: &[String],
) -> (PolicyVerdict, String) {
    let deny_match = best_prefix_match(deny, subject);
    let allow_match = best_prefix_match(allow, subject);
    evaluate_matches(scope, subject_kind, subject, allow_match, deny_match)
}

fn evaluate_matches(
    scope: &str,
    subject_kind: &str,
    subject: &str,
    allow_match: Option<RuleMatch>,
    deny_match: Option<RuleMatch>,
) -> (PolicyVerdict, String) {
    if let Some(m) = deny_match {
        return (
            PolicyVerdict::Deny,
            format!(
                "denied: {scope} matched deny rule `{}` for {subject_kind} `{subject}` (specificity={}, rule_index={})",
                m.pattern, m.specificity, m.declaration_index
            ),
        );
    }
    if let Some(m) = allow_match {
        return (
            PolicyVerdict::Allow,
            format!(
                "allowed: {scope} matched allow rule `{}` for {subject_kind} `{subject}` (specificity={}, rule_index={})",
                m.pattern, m.specificity, m.declaration_index
            ),
        );
    }
    (
        PolicyVerdict::Deny,
        format!("denied: no {scope} allow rule matched {subject_kind} `{subject}`"),
    )
}

fn best_pattern_match(patterns: &[String], subject: &str) -> Option<RuleMatch> {
    patterns
        .iter()
        .enumerate()
        .filter_map(|(idx, pattern)| {
            let parsed = Pattern::new(pattern).ok()?;
            if !parsed.matches(subject) {
                return None;
            }
            Some(RuleMatch {
                pattern: pattern.clone(),
                specificity: pattern_specificity(pattern),
                declaration_index: idx,
            })
        })
        .max_by(|a, b| {
            a.specificity
                .cmp(&b.specificity)
                .then_with(|| b.declaration_index.cmp(&a.declaration_index))
        })
}

fn best_prefix_match(prefixes: &[String], subject: &str) -> Option<RuleMatch> {
    prefixes
        .iter()
        .enumerate()
        .filter_map(|(idx, prefix)| {
            let normalized = prefix.trim();
            if normalized.is_empty() || !subject.starts_with(normalized) {
                return None;
            }
            Some(RuleMatch {
                pattern: normalized.to_string(),
                specificity: normalized.len(),
                declaration_index: idx,
            })
        })
        .max_by(|a, b| {
            a.specificity
                .cmp(&b.specificity)
                .then_with(|| b.declaration_index.cmp(&a.declaration_index))
        })
}

fn pattern_specificity(pattern: &str) -> usize {
    pattern
        .chars()
        .filter(|ch| !matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | '!'))
        .count()
}

fn normalize_path_for_policy(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn extract_domain(url: &str) -> Option<String> {
    let without_scheme = if let Some((_, rest)) = url.split_once("://") {
        rest
    } else {
        url
    };
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.is_empty() {
        return None;
    }
    let host = authority
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .map(|v| v.to_string())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or_default().to_string());
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

/// How the engine decides automatic permissions (anything other than a live
/// caller verdict in interactive mode).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEngineMode {
    /// Default: deny every automatic evaluation (safest out-of-the-box).
    #[default]
    DenyAll,
    /// Caller must supply allow/deny per action via [`PolicyEngine::resolve_interactive`].
    Interactive,
    /// Automatically allow read-only filesystem introspection; writes and [`Permission::ExecuteCommand`]
    /// still require interactive confirmation when `confirm_writes` is true (shell is never auto-approved).
    AutoApproveReads {
        /// When true, [`Permission::WriteFile`] and [`Permission::ExecuteCommand`] use interactive confirmation instead of automatic denial.
        confirm_writes: bool,
    },
    /// Like [`Self::AutoApproveReads`], but also auto-allows [`Permission::NetworkFetch`] (SSRF checks still run in the tool before execution).
    ///
    /// [`Permission::WriteFile`] still uses interactive confirmation when `confirm_writes` is true.
    /// [`Permission::ExecuteCommand`] is never auto-approved (same confirmation path as [`Self::AutoApproveReads`]).
    AutoApproveReadsAndFetch {
        /// When true, [`Permission::WriteFile`] uses interactive confirmation instead of automatic denial.
        confirm_writes: bool,
    },
    /// Rules from [`PolicyConfig`].
    Configured(PolicyConfig),
}

/// Error returned when the engine is used in a way that does not match its mode.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyEngineError {
    /// [`PolicyEngineMode::Interactive`] (or write confirmation under [`PolicyEngineMode::AutoApproveReads`]
    /// or [`PolicyEngineMode::AutoApproveReadsAndFetch`]) does not support automatic resolution for this permission.
    #[error(
        "interactive confirmation required; use resolve_interactive after the user supplies a verdict"
    )]
    InteractiveRequiresCaller,
    /// [`PolicyEngine::resolve_interactive`] was called while not in interactive or auto-read-approve mode.
    #[error(
        "resolve_interactive is only valid in Interactive, AutoApproveReads, or AutoApproveReadsAndFetch mode"
    )]
    NotInteractive,
}

/// Result of policy evaluation: verdict, explanation, and audit record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    /// Whether the permission is granted.
    pub allowed: bool,
    /// Explanation suitable for UI and logs (no secrets).
    pub reason: String,
    /// Audit record for this evaluation (always produced).
    pub audit: AuditEvent,
}

/// Evaluates [`Permission`] requests and emits [`AuditEvent::PolicyEvaluation`] for each decision.
#[derive(Debug, Clone)]
pub struct PolicyEngine {
    mode: PolicyEngineMode,
}

impl PolicyEngine {
    /// Creates an engine with the given [`PolicyEngineMode`].
    pub fn new(mode: PolicyEngineMode) -> Self {
        Self { mode }
    }

    /// Returns the active mode.
    pub fn mode(&self) -> &PolicyEngineMode {
        &self.mode
    }

    /// Automatic evaluation: [`PolicyEngineMode::DenyAll`], [`PolicyEngineMode::Configured`],
    /// [`PolicyEngineMode::AutoApproveReads`], [`PolicyEngineMode::AutoApproveReadsAndFetch`],
    /// and the allow branch of [`PolicyEngineMode::Interactive`] (which always
    /// signals that the caller must prompt).
    ///
    /// Returns [`PolicyEngineError::InteractiveRequiresCaller`] when the mode is
    /// [`PolicyEngineMode::Interactive`], or [`PolicyEngineMode::AutoApproveReads`] /
    /// [`PolicyEngineMode::AutoApproveReadsAndFetch`] with `confirm_writes: true` and a
    /// [`Permission::WriteFile`], or [`Permission::ExecuteCommand`] (shell is never auto-approved).
    pub fn evaluate_automatic(
        &self,
        session_id: &str,
        permission: Permission,
    ) -> Result<PolicyDecision, PolicyEngineError> {
        self.evaluate_automatic_for_tool(session_id, permission, None)
    }

    /// Automatic evaluation with optional `tool_name` context.
    ///
    /// When provided in [`PolicyEngineMode::Configured`], tool-level rules are
    /// checked first and can deny before permission-specific rules.
    pub fn evaluate_automatic_for_tool(
        &self,
        session_id: &str,
        permission: Permission,
        tool_name: Option<&str>,
    ) -> Result<PolicyDecision, PolicyEngineError> {
        let timestamp = Utc::now();
        let (allowed, reason, verdict) = match &self.mode {
            PolicyEngineMode::DenyAll => (
                false,
                "denied: PolicyEngineMode::DenyAll".to_string(),
                PolicyVerdict::Deny,
            ),
            PolicyEngineMode::Interactive => {
                return Err(PolicyEngineError::InteractiveRequiresCaller);
            }
            PolicyEngineMode::AutoApproveReads { confirm_writes } => match &permission {
                Permission::ReadFile { .. } | Permission::ListDirectory { .. } => (
                    true,
                    "auto-approved read (--yes)".to_string(),
                    PolicyVerdict::Allow,
                ),
                Permission::NetworkFetch { .. } => (
                    false,
                    "denied: web fetch is never auto-approved (--yes)".to_string(),
                    PolicyVerdict::Deny,
                ),
                Permission::WriteFile { .. } => {
                    if *confirm_writes {
                        return Err(PolicyEngineError::InteractiveRequiresCaller);
                    }
                    (
                        false,
                        "requires confirmation".to_string(),
                        PolicyVerdict::Deny,
                    )
                }
                Permission::ExecuteCommand { .. } => {
                    return Err(PolicyEngineError::InteractiveRequiresCaller);
                }
            },
            PolicyEngineMode::AutoApproveReadsAndFetch { confirm_writes } => match &permission {
                Permission::ReadFile { .. } | Permission::ListDirectory { .. } => (
                    true,
                    "auto-approved read (--yes)".to_string(),
                    PolicyVerdict::Allow,
                ),
                Permission::NetworkFetch { .. } => (
                    true,
                    "auto-approved web fetch (--yes-web)".to_string(),
                    PolicyVerdict::Allow,
                ),
                Permission::WriteFile { .. } => {
                    if *confirm_writes {
                        return Err(PolicyEngineError::InteractiveRequiresCaller);
                    }
                    (
                        false,
                        "requires confirmation".to_string(),
                        PolicyVerdict::Deny,
                    )
                }
                Permission::ExecuteCommand { .. } => {
                    return Err(PolicyEngineError::InteractiveRequiresCaller);
                }
            },
            PolicyEngineMode::Configured(cfg) => {
                if let Some(name) = tool_name {
                    if cfg.has_tool_rules() {
                        let (tool_verdict, tool_reason) = cfg.evaluate_tool_name(name);
                        if matches!(tool_verdict, PolicyVerdict::Deny) {
                            (
                                false,
                                format!(
                                    "denied: tool `{name}` blocked before permission check ({tool_reason})"
                                ),
                                PolicyVerdict::Deny,
                            )
                        } else {
                            let (verdict, permission_reason) = cfg.evaluate_permission(&permission);
                            let allowed = matches!(verdict, PolicyVerdict::Allow);
                            (
                                allowed,
                                format!(
                                    "tool `{name}` passed tool policy ({tool_reason}); {permission_reason}"
                                ),
                                verdict,
                            )
                        }
                    } else {
                        let (verdict, r) = cfg.evaluate_permission(&permission);
                        let allowed = matches!(verdict, PolicyVerdict::Allow);
                        (allowed, r, verdict)
                    }
                } else {
                    let (verdict, r) = cfg.evaluate_permission(&permission);
                    let allowed = matches!(verdict, PolicyVerdict::Allow);
                    (allowed, r, verdict)
                }
            }
        };

        Ok(Self::decision_from_parts(
            session_id, timestamp, permission, verdict, allowed, reason,
        ))
    }

    /// Records a caller-supplied verdict in [`PolicyEngineMode::Interactive`],
    /// [`PolicyEngineMode::AutoApproveReads`], or [`PolicyEngineMode::AutoApproveReadsAndFetch`]
    /// (e.g. write confirmation after `--yes`). Always emits an audit event.
    pub fn resolve_interactive(
        &self,
        session_id: &str,
        permission: Permission,
        verdict: PolicyVerdict,
        reason: impl Into<String>,
    ) -> Result<PolicyDecision, PolicyEngineError> {
        if !matches!(
            self.mode,
            PolicyEngineMode::Interactive
                | PolicyEngineMode::AutoApproveReads { .. }
                | PolicyEngineMode::AutoApproveReadsAndFetch { .. }
        ) {
            return Err(PolicyEngineError::NotInteractive);
        }
        let reason = reason.into();
        let allowed = matches!(verdict, PolicyVerdict::Allow);
        let timestamp = Utc::now();
        Ok(Self::decision_from_parts(
            session_id, timestamp, permission, verdict, allowed, reason,
        ))
    }

    fn decision_from_parts(
        session_id: &str,
        timestamp: chrono::DateTime<Utc>,
        permission: Permission,
        verdict: PolicyVerdict,
        allowed: bool,
        reason: String,
    ) -> PolicyDecision {
        let audit = AuditEvent::PolicyEvaluation {
            session_id: session_id.to_string(),
            timestamp,
            permission,
            verdict,
            reason: reason.clone(),
        };
        PolicyDecision {
            allowed,
            reason,
            audit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn deny_all_emits_audit() {
        let engine = PolicyEngine::new(PolicyEngineMode::DenyAll);
        let perm = Permission::ReadFile {
            path: PathBuf::from("README.md"),
        };
        let decision = engine.evaluate_automatic("sess-1", perm).unwrap();
        assert!(!decision.allowed);
        let json = decision.audit.to_json().expect("audit json");
        assert!(json.contains("sess-1"));
        assert!(json.contains("deny"));
    }

    #[test]
    fn configured_denies_when_no_allow_matches() {
        let cfg = PolicyConfig {
            network: NetworkPolicyConfig {
                allow_domains: vec!["api.example.com".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::NetworkFetch {
            url: "https://example.com".into(),
        };
        let decision = engine
            .evaluate_automatic("sess-c", perm)
            .expect("configured decision");
        assert!(!decision.allowed);
        assert!(decision.reason.contains("no network allow rule"));
    }

    #[test]
    fn configured_allows_when_allow_rule_matches() {
        let cfg = PolicyConfig {
            network: NetworkPolicyConfig {
                allow_domains: vec!["*.example.com".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::NetworkFetch {
            url: "https://docs.example.com/v1".into(),
        };
        let decision = engine
            .evaluate_automatic("sess-c", perm)
            .expect("configured decision");
        assert!(decision.allowed);
        assert!(decision.reason.contains("matched allow rule"));
    }

    #[test]
    fn configured_deny_wins_over_allow_for_path() {
        let cfg = PolicyConfig {
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["src/**".into()],
                    deny: vec!["src/secret/**".into()],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::ReadFile {
            path: PathBuf::from("src/secret/token.txt"),
        };
        let decision = engine
            .evaluate_automatic("sess-fs", perm)
            .expect("configured decision");
        assert!(!decision.allowed);
        assert!(decision.reason.contains("matched deny rule"));
    }

    #[test]
    fn configured_deny_wins_even_when_allow_is_more_specific() {
        let cfg = PolicyConfig {
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["src/security/private.txt".into()],
                    deny: vec!["src/**".into()],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::ReadFile {
            path: PathBuf::from("src/security/private.txt"),
        };
        let decision = engine
            .evaluate_automatic("sess-fs", perm)
            .expect("configured decision");
        assert!(!decision.allowed);
        assert!(decision.reason.contains("matched deny rule"));
    }

    #[test]
    fn configured_uses_most_specific_allow_rule() {
        let cfg = PolicyConfig {
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["src/**".into(), "src/security/**".into()],
                    deny: vec![],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::ReadFile {
            path: PathBuf::from("src/security/policy.rs"),
        };
        let decision = engine
            .evaluate_automatic("sess-fs", perm)
            .expect("configured decision");
        assert!(decision.allowed);
        assert!(decision.reason.contains("src/security/**"));
    }

    #[test]
    fn configured_shell_prefix_deny_wins() {
        let cfg = PolicyConfig {
            shell: ShellPolicyConfig {
                allow_prefixes: vec!["cargo ".into()],
                deny_prefixes: vec!["cargo publish".into()],
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::ExecuteCommand {
            command: "cargo publish --dry-run".into(),
            cwd: PathBuf::from("."),
        };
        let decision = engine
            .evaluate_automatic("sess-shell", perm)
            .expect("configured decision");
        assert!(!decision.allowed);
        assert!(decision.reason.contains("cargo publish"));
    }

    #[test]
    fn configured_network_rule_uses_domain_not_full_url() {
        let cfg = PolicyConfig {
            network: NetworkPolicyConfig {
                allow_domains: vec!["api.example.com".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let perm = Permission::NetworkFetch {
            url: "https://api.example.com/v1/users".into(),
        };
        let decision = engine
            .evaluate_automatic("sess-net", perm)
            .expect("configured decision");
        assert!(decision.allowed);
    }

    #[test]
    fn configured_tool_name_rules_are_evaluated() {
        let cfg = PolicyConfig {
            tools: ToolPolicyConfig {
                allow: vec!["shell*".into()],
                deny: vec!["shell_dangerous".into()],
            },
            ..PolicyConfig::default()
        };
        let (v1, r1) = cfg.evaluate_tool_name("shell");
        assert_eq!(v1, PolicyVerdict::Allow);
        assert!(r1.contains("matched allow rule"));

        let (v2, r2) = cfg.evaluate_tool_name("shell_dangerous");
        assert_eq!(v2, PolicyVerdict::Deny);
        assert!(r2.contains("matched deny rule"));
    }

    #[test]
    fn configured_tool_rules_apply_during_automatic_evaluation() {
        let cfg = PolicyConfig {
            tools: ToolPolicyConfig {
                allow: vec!["read_*".into()],
                deny: vec!["shell".into()],
            },
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["README.md".into()],
                    deny: vec![],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));

        let denied = engine
            .evaluate_automatic_for_tool(
                "sess-tool",
                Permission::ReadFile {
                    path: PathBuf::from("README.md"),
                },
                Some("shell"),
            )
            .expect("configured decision");
        assert!(!denied.allowed);
        assert!(denied.reason.contains("blocked before permission check"));

        let allowed = engine
            .evaluate_automatic_for_tool(
                "sess-tool",
                Permission::ReadFile {
                    path: PathBuf::from("README.md"),
                },
                Some("read_file"),
            )
            .expect("configured decision");
        assert!(allowed.allowed);
        assert!(allowed.reason.contains("passed tool policy"));
    }

    #[test]
    fn non_tool_policy_evaluation_path_preserves_existing_behavior() {
        let cfg = PolicyConfig {
            tools: ToolPolicyConfig {
                allow: vec!["read_*".into()],
                deny: vec!["read_file".into()],
            },
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["README.md".into()],
                    deny: vec![],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let decision = engine
            .evaluate_automatic(
                "sess-legacy",
                Permission::ReadFile {
                    path: PathBuf::from("README.md"),
                },
            )
            .expect("configured decision");
        assert!(decision.allowed);
        assert!(
            !decision.reason.contains("tool"),
            "legacy non-tool path should not include tool gating reason"
        );
    }

    #[test]
    fn interactive_automatic_fails() {
        let engine = PolicyEngine::new(PolicyEngineMode::Interactive);
        let perm = Permission::ReadFile {
            path: PathBuf::from("x"),
        };
        assert_eq!(
            engine.evaluate_automatic("s", perm),
            Err(PolicyEngineError::InteractiveRequiresCaller)
        );
    }

    #[test]
    fn interactive_resolve_emits_allow() {
        let engine = PolicyEngine::new(PolicyEngineMode::Interactive);
        let perm = Permission::ReadFile {
            path: PathBuf::from("README.md"),
        };
        let decision = engine
            .resolve_interactive("sess-i", perm, PolicyVerdict::Allow, "user approved in TUI")
            .unwrap();
        assert!(decision.allowed);
        assert!(
            decision
                .audit
                .to_json()
                .expect("audit json")
                .contains("allow")
        );
    }

    #[test]
    fn resolve_interactive_wrong_mode_fails() {
        let engine = PolicyEngine::new(PolicyEngineMode::DenyAll);
        let perm = Permission::ReadFile {
            path: PathBuf::from("x"),
        };
        assert_eq!(
            engine.resolve_interactive("s", perm, PolicyVerdict::Allow, "n/a"),
            Err(PolicyEngineError::NotInteractive)
        );
    }

    #[test]
    fn auto_approve_reads_allows_read_and_list() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let read = Permission::ReadFile {
            path: PathBuf::from("x"),
        };
        let list = Permission::ListDirectory {
            path: PathBuf::from("."),
        };
        assert!(engine.evaluate_automatic("s", read).unwrap().allowed);
        assert!(engine.evaluate_automatic("s", list).unwrap().allowed);
    }

    #[test]
    fn auto_approve_reads_write_needs_caller() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let perm = Permission::WriteFile {
            path: PathBuf::from("out.txt"),
        };
        assert_eq!(
            engine.evaluate_automatic("s", perm),
            Err(PolicyEngineError::InteractiveRequiresCaller)
        );
    }

    #[test]
    fn auto_approve_reads_network_fetch_denied() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let perm = Permission::NetworkFetch {
            url: "https://x.test".into(),
        };
        let d = engine.evaluate_automatic("s", perm).unwrap();
        assert!(!d.allowed);
        assert!(
            d.reason.contains("web fetch"),
            "expected web-fetch denial reason, got {:?}",
            d.reason
        );
    }

    #[test]
    fn auto_approve_reads_resolve_interactive_ok() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let perm = Permission::WriteFile {
            path: PathBuf::from("f"),
        };
        let d = engine
            .resolve_interactive("s", perm, PolicyVerdict::Allow, "ok")
            .unwrap();
        assert!(d.allowed);
    }

    #[test]
    fn auto_approve_reads_execute_command_never_auto_approved() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let perm = Permission::ExecuteCommand {
            command: "cargo test".into(),
            cwd: PathBuf::from("/tmp"),
        };
        assert_eq!(
            engine.evaluate_automatic("s", perm),
            Err(PolicyEngineError::InteractiveRequiresCaller)
        );
    }

    #[test]
    fn auto_approve_reads_and_fetch_allows_network_fetch() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReadsAndFetch {
            confirm_writes: true,
        });
        let perm = Permission::NetworkFetch {
            url: "https://example.com".into(),
        };
        let d = engine.evaluate_automatic("s", perm).unwrap();
        assert!(d.allowed);
        assert!(
            d.reason.contains("yes-web"),
            "expected yes-web in reason, got {:?}",
            d.reason
        );
    }

    #[test]
    fn auto_approve_reads_and_fetch_execute_command_never_auto_approved() {
        let engine = PolicyEngine::new(PolicyEngineMode::AutoApproveReadsAndFetch {
            confirm_writes: true,
        });
        let perm = Permission::ExecuteCommand {
            command: "cargo test".into(),
            cwd: PathBuf::from("/tmp"),
        };
        assert_eq!(
            engine.evaluate_automatic("s", perm),
            Err(PolicyEngineError::InteractiveRequiresCaller)
        );
    }
}
