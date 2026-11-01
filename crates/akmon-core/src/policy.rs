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
    /// MCP server/tool governance rules for external tool calls.
    pub mcp: McpPolicyConfig,
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

/// MCP governance rules split by server and tool dimensions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpPolicyConfig {
    /// MCP server allow/deny patterns matched against server name.
    pub servers: PatternRuleSet,
    /// MCP tool allow/deny patterns matched against MCP tool name.
    pub tools: PatternRuleSet,
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
                let subject = match normalize_path_for_policy(path) {
                    Ok(s) => s,
                    Err(why) => return deny_unnormalizable_path("filesystem.read", path, why),
                };
                evaluate_pattern_rules(
                    "filesystem.read",
                    "path",
                    &subject,
                    &self.filesystem.read.allow,
                    &self.filesystem.read.deny,
                )
            }
            Permission::WriteFile { path } => {
                let subject = match normalize_path_for_policy(path) {
                    Ok(s) => s,
                    Err(why) => return deny_unnormalizable_path("filesystem.write", path, why),
                };
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

    /// Applies MCP server/tool governance rules.
    ///
    /// Fail-closed behavior:
    /// - missing or empty `server_name` / `tool_name` denies;
    /// - explicit deny wins;
    /// - no allow match denies.
    pub fn evaluate_mcp_action(
        &self,
        server_name: Option<&str>,
        tool_name: Option<&str>,
    ) -> (PolicyVerdict, String) {
        let server = server_name.map(str::trim).unwrap_or_default();
        let tool = tool_name.map(str::trim).unwrap_or_default();
        if server.is_empty() || tool.is_empty() {
            return (
                PolicyVerdict::Deny,
                "denied: malformed MCP context (missing server or tool)".to_string(),
            );
        }
        let (server_verdict, server_reason) = evaluate_pattern_rules(
            "mcp.server",
            "server",
            server,
            &self.mcp.servers.allow,
            &self.mcp.servers.deny,
        );
        if matches!(server_verdict, PolicyVerdict::Deny) {
            return (
                PolicyVerdict::Deny,
                format!("denied: MCP server policy blocked `{server}` ({server_reason})"),
            );
        }
        let (tool_verdict, tool_reason) = evaluate_pattern_rules(
            "mcp.tool",
            "tool",
            tool,
            &self.mcp.tools.allow,
            &self.mcp.tools.deny,
        );
        if matches!(tool_verdict, PolicyVerdict::Deny) {
            return (
                PolicyVerdict::Deny,
                format!("denied: MCP tool policy blocked `{tool}` ({tool_reason})"),
            );
        }
        (
            PolicyVerdict::Allow,
            format!(
                "allowed: MCP context `{server}` / `{tool}` passed server+tool rules ({server_reason}; {tool_reason})"
            ),
        )
    }

    fn has_tool_rules(&self) -> bool {
        !self.tools.allow.is_empty() || !self.tools.deny.is_empty()
    }

    fn has_mcp_rules(&self) -> bool {
        !self.mcp.servers.allow.is_empty()
            || !self.mcp.servers.deny.is_empty()
            || !self.mcp.tools.allow.is_empty()
            || !self.mcp.tools.deny.is_empty()
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

/// Why a filesystem path could not be turned into a policy subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathNormalizeError {
    /// A relative path whose `..` components climb above the project root, evaluated without a
    /// workspace root to resolve it against (see [`policy_subject_path`], which the agent session
    /// applies first so this only happens on direct, root-less evaluation).
    EscapesRoot,
}

impl PathNormalizeError {
    fn describe(self) -> &'static str {
        match self {
            Self::EscapesRoot => {
                "path climbs above the project root via `..` and no workspace root is known to resolve it"
            }
        }
    }
}

fn deny_unnormalizable_path(
    scope: &str,
    path: &std::path::Path,
    why: PathNormalizeError,
) -> (PolicyVerdict, String) {
    (
        PolicyVerdict::Deny,
        format!(
            "denied: {scope} path `{}`: {}",
            path.to_string_lossy(),
            why.describe()
        ),
    )
}

/// Splits a `/`-separated absolute path into its root prefix (`/`, `//server/share/`, `C:/`) and
/// the rest, or returns `None` for a relative path.
fn split_absolute_prefix(raw: &str) -> Option<(String, &str)> {
    if let Some(rest) = raw.strip_prefix("//") {
        // UNC: `//server/share/...`
        let mut it = rest.splitn(3, '/');
        let server = it.next().unwrap_or_default();
        let share = it.next().unwrap_or_default();
        let tail = it.next().unwrap_or_default();
        return Some((format!("//{server}/{share}/"), tail));
    }
    if let Some(rest) = raw.strip_prefix('/') {
        return Some(("/".to_string(), rest));
    }
    if let [drive, b':', ..] = raw.as_bytes()
        && drive.is_ascii_alphabetic()
    {
        // `C:/x` and drive-relative `C:x` are both treated as absolute on that drive.
        let rest = raw[2..].trim_start_matches('/');
        return Some((format!("{}:/", char::from(*drive)), rest));
    }
    None
}

/// Lexically normalizes a model-supplied path into the canonical form that policy globs are
/// matched against, **before** any allow/deny matching.
///
/// Backslashes become `/`; empty and `.` segments are dropped; `..` pops the previous segment.
///
/// - A relative path stays relative to the project root: `src/../.github/x` is evaluated as
///   `.github/x`, `./.git/config` as `.git/config`, and the root itself (including the empty
///   path, which tools use as their static placeholder permission) as `.`.
/// - An absolute path (`/abs/x`, `//server/share/x`, `C:/x`) stays absolute and normalized
///   (`..` at the filesystem root is a no-op); it is matched against the globs as-is, so
///   `allow = ["**"]` or an explicit absolute glob can allow it and deny globs still apply.
///   Callers that know the workspace root run [`policy_subject_path`] first, which turns an
///   absolute path INSIDE the root back into a relative one.
/// - A relative path whose `..` climbs above the root is an error (callers deny) because it
///   cannot be resolved without a root; [`policy_subject_path`] resolves it to an absolute path.
fn normalize_path_for_policy(path: &std::path::Path) -> Result<String, PathNormalizeError> {
    let raw = path.to_string_lossy().replace('\\', "/");
    let (prefix, rest) = match split_absolute_prefix(&raw) {
        Some((prefix, rest)) => (Some(prefix), rest),
        None => (None, raw.as_str()),
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() && prefix.is_none() {
                    return Err(PathNormalizeError::EscapesRoot);
                }
            }
            s => parts.push(s),
        }
    }
    Ok(match prefix {
        Some(prefix) => format!("{prefix}{}", parts.join("/")),
        None if parts.is_empty() => ".".to_string(),
        None => parts.join("/"),
    })
}

/// Maps a model-supplied path to the policy subject for a workspace rooted at `workspace_root`.
///
/// - A relative path that stays inside the root is returned unchanged (the policy normalizes it
///   lexically when evaluating, so `src/../X` is still matched as `X`).
/// - An absolute path inside the root (lexically, or under the root's canonical form, so
///   `/private/var/...` vs `/var/...` both work) is relativized: `/repo/src/lib.rs` becomes
///   `src/lib.rs`, and the root itself becomes `.`.
/// - Anything outside the root — an absolute path into an `--add-dir` root, or a relative path
///   whose `..` escapes the root (resolved lexically against the root) — becomes a normalized
///   absolute path, which [`PolicyConfig::evaluate_permission`] matches against the globs as-is.
///
/// Purely lexical apart from canonicalizing `workspace_root` itself; symlinks inside `path` are
/// not resolved (the sandbox handles containment separately).
#[must_use]
pub fn policy_subject_path(
    path: &std::path::Path,
    workspace_root: &std::path::Path,
) -> std::path::PathBuf {
    let subject = match normalize_path_for_policy(path) {
        Ok(s) if split_absolute_prefix(&s).is_none() => return path.to_path_buf(),
        Ok(abs) => abs,
        Err(PathNormalizeError::EscapesRoot) => {
            match normalize_path_for_policy(&workspace_root.join(path)) {
                Ok(s) => s,
                Err(_) => return path.to_path_buf(),
            }
        }
    };
    let mut roots: Vec<String> = Vec::with_capacity(2);
    if let Ok(r) = normalize_path_for_policy(workspace_root) {
        roots.push(r);
    }
    if let Ok(canonical) = std::fs::canonicalize(workspace_root)
        && let Ok(r) = normalize_path_for_policy(&canonical)
        && !roots.contains(&r)
    {
        roots.push(r);
    }
    for root in roots.iter().filter(|r| split_absolute_prefix(r).is_some()) {
        if subject == *root {
            return std::path::PathBuf::from(".");
        }
        let with_sep = if root.ends_with('/') {
            root.clone()
        } else {
            format!("{root}/")
        };
        if let Some(rel) = subject.strip_prefix(with_sep.as_str()) {
            return std::path::PathBuf::from(if rel.is_empty() { "." } else { rel });
        }
    }
    std::path::PathBuf::from(subject)
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

    /// Automatic MCP governance evaluation (server + tool context).
    ///
    /// This path is intentionally fail-closed. MCP execution requires
    /// [`PolicyEngineMode::Configured`], and both MCP server and tool context.
    pub fn evaluate_mcp_automatic(
        &self,
        session_id: &str,
        dispatch_tool_name: Option<&str>,
        mcp_server: Option<&str>,
        mcp_tool: Option<&str>,
    ) -> PolicyDecision {
        let timestamp = Utc::now();
        let (allowed, reason, verdict) = match &self.mode {
            PolicyEngineMode::Configured(cfg) => {
                let server = mcp_server.map(str::trim).unwrap_or_default();
                let tool = mcp_tool.map(str::trim).unwrap_or_default();
                if server.is_empty() || tool.is_empty() {
                    (
                        false,
                        "denied: malformed MCP context (missing server or tool)".to_string(),
                        PolicyVerdict::Deny,
                    )
                } else if !cfg.has_mcp_rules() {
                    (
                        false,
                        "denied: MCP policy missing (configured mode requires explicit [mcp] allow rules)"
                            .to_string(),
                        PolicyVerdict::Deny,
                    )
                } else {
                    let tool_gate = dispatch_tool_name.and_then(|name| {
                        if cfg.has_tool_rules() {
                            Some((name, cfg.evaluate_tool_name(name)))
                        } else {
                            None
                        }
                    });
                    if let Some((name, (tool_verdict, tool_reason))) = tool_gate {
                        if matches!(tool_verdict, PolicyVerdict::Deny) {
                            (
                                false,
                                format!(
                                    "denied: parent tool policy blocked dispatch tool `{name}` ({tool_reason})"
                                ),
                                PolicyVerdict::Deny,
                            )
                        } else {
                            let (mcp_verdict, mcp_reason) =
                                cfg.evaluate_mcp_action(Some(server), Some(tool));
                            (
                                matches!(mcp_verdict, PolicyVerdict::Allow),
                                format!(
                                    "dispatch tool `{name}` passed parent tool policy ({tool_reason}); {mcp_reason}"
                                ),
                                mcp_verdict,
                            )
                        }
                    } else {
                        let (mcp_verdict, mcp_reason) =
                            cfg.evaluate_mcp_action(Some(server), Some(tool));
                        (matches!(mcp_verdict, PolicyVerdict::Allow), mcp_reason, mcp_verdict)
                    }
                }
            }
            PolicyEngineMode::DenyAll => (
                false,
                "denied: parent policy mode is deny-all".to_string(),
                PolicyVerdict::Deny,
            ),
            PolicyEngineMode::Interactive => (
                false,
                "denied: parent policy mode is interactive; MCP requires configured fail-closed rules"
                    .to_string(),
                PolicyVerdict::Deny,
            ),
            PolicyEngineMode::AutoApproveReads { .. } => (
                false,
                "denied: parent policy mode is auto-approve-reads; MCP requires configured fail-closed rules"
                    .to_string(),
                PolicyVerdict::Deny,
            ),
            PolicyEngineMode::AutoApproveReadsAndFetch { .. } => (
                false,
                "denied: parent policy mode is auto-approve-reads-and-fetch; MCP requires configured fail-closed rules"
                    .to_string(),
                PolicyVerdict::Deny,
            ),
        };
        let audit = AuditEvent::PolicyEvaluation {
            session_id: session_id.to_string(),
            timestamp,
            permission: Permission::NetworkFetch {
                url: "mcp://tool-call".to_string(),
            },
            verdict,
            reason: reason.clone(),
            mcp_server: mcp_server.map(ToOwned::to_owned),
            mcp_tool: mcp_tool.map(ToOwned::to_owned),
            decision_reason: Some(reason.clone()),
        };
        PolicyDecision {
            allowed,
            reason,
            audit,
        }
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
            mcp_server: None,
            mcp_tool: None,
            decision_reason: None,
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

    #[test]
    fn configured_mcp_requires_explicit_rules_fail_closed() {
        let cfg = PolicyConfig::default();
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let decision = engine.evaluate_mcp_automatic(
            "sess-mcp",
            Some("mcp_tool"),
            Some("github"),
            Some("search_issues"),
        );
        assert!(!decision.allowed);
        assert!(decision.reason.contains("MCP policy missing"));
    }

    #[test]
    fn configured_mcp_denies_malformed_context() {
        let cfg = PolicyConfig {
            mcp: McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec![],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec![],
                },
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let decision = engine.evaluate_mcp_automatic("sess-mcp", Some("mcp_tool"), None, Some("x"));
        assert!(!decision.allowed);
        assert!(decision.reason.contains("malformed MCP context"));
    }

    #[test]
    fn configured_mcp_server_or_tool_deny_wins() {
        let cfg = PolicyConfig {
            mcp: McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec!["github".into()],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec!["search_secret".into()],
                },
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let server_denied = engine.evaluate_mcp_automatic(
            "sess-mcp",
            Some("mcp_tool"),
            Some("github"),
            Some("search_issues"),
        );
        assert!(!server_denied.allowed);
        assert!(server_denied.reason.contains("MCP server policy blocked"));
    }

    #[test]
    fn configured_mcp_allows_when_server_and_tool_match_allow() {
        let cfg = PolicyConfig {
            mcp: McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec![],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec![],
                },
            },
            ..PolicyConfig::default()
        };
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let decision = engine.evaluate_mcp_automatic(
            "sess-mcp",
            Some("mcp_tool"),
            Some("github"),
            Some("search_issues"),
        );
        assert!(decision.allowed);
        let json = decision.audit.to_json().expect("policy audit json");
        assert!(json.contains("\"mcp_server\":\"github\""));
        assert!(json.contains("\"mcp_tool\":\"search_issues\""));
        assert!(json.contains("decision_reason"));
    }

    // ---- AGT-03: paths are normalized before glob matching ----

    fn dev_cfg() -> PolicyConfig {
        crate::policy_profiles::built_in_policy_profile(
            crate::policy_profiles::PolicyProfileName::Dev,
        )
    }

    fn write(cfg: &PolicyConfig, p: &str) -> (PolicyVerdict, String) {
        cfg.evaluate_permission(&Permission::WriteFile {
            path: PathBuf::from(p),
        })
    }

    fn read(cfg: &PolicyConfig, p: &str) -> (PolicyVerdict, String) {
        cfg.evaluate_permission(&Permission::ReadFile {
            path: PathBuf::from(p),
        })
    }

    #[test]
    fn normalize_resolves_dot_and_dotdot_lexically() {
        let n = |p: &str| normalize_path_for_policy(std::path::Path::new(p));
        assert_eq!(n("src/../X"), Ok("X".to_string()));
        assert_eq!(n("./.git/config"), Ok(".git/config".to_string()));
        assert_eq!(n("src//a/./b/"), Ok("src/a/b".to_string()));
        assert_eq!(n("src\\..\\.git\\HEAD"), Ok(".git/HEAD".to_string()));
        assert_eq!(n("."), Ok(".".to_string()));
        assert_eq!(n(""), Ok(".".to_string()));
        assert_eq!(n("src/.."), Ok(".".to_string()));
        assert_eq!(n("../x"), Err(PathNormalizeError::EscapesRoot));
        assert_eq!(n("src/../../x"), Err(PathNormalizeError::EscapesRoot));
        assert_eq!(n("/etc/passwd"), Ok("/etc/passwd".to_string()));
        assert_eq!(n("/a/./b/../c"), Ok("/a/c".to_string()));
        assert_eq!(n("/.."), Ok("/".to_string()));
        assert_eq!(n("C:/Windows"), Ok("C:/Windows".to_string()));
        assert_eq!(n("c:foo"), Ok("c:/foo".to_string()));
        assert_eq!(
            n("\\\\server\\share\\x"),
            Ok("//server/share/x".to_string())
        );
    }

    /// Tools declare `PathBuf::new()` as their static placeholder permission; it is evaluated as
    /// the project root `.` (so `**`-style allow rules match it and `src/**` does not).
    #[test]
    fn empty_placeholder_path_is_evaluated_as_project_root() {
        let cfg = dev_cfg();
        let (rv, rr) = cfg.evaluate_permission(&Permission::ReadFile {
            path: PathBuf::new(),
        });
        assert_eq!(rv, PolicyVerdict::Allow, "{rr}");
        assert!(rr.contains("path `.`"), "{rr}");
        let (wv, wr) = cfg.evaluate_permission(&Permission::WriteFile {
            path: PathBuf::new(),
        });
        assert_eq!(wv, PolicyVerdict::Deny, "{wr}");
        assert!(wr.contains("path `.`"), "{wr}");
    }

    #[test]
    fn policy_subject_relativizes_inside_and_absolutizes_outside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("mkdir");
        let root_s = root.to_string_lossy().replace('\\', "/");
        let sub = |p: &str| {
            policy_subject_path(std::path::Path::new(p), &root)
                .to_string_lossy()
                .replace('\\', "/")
        };
        assert_eq!(sub("src/lib.rs"), "src/lib.rs");
        assert_eq!(sub("./src/../.git/config"), "./src/../.git/config");
        assert_eq!(sub(&format!("{root_s}/src/lib.rs")), "src/lib.rs");
        assert_eq!(sub(&format!("{root_s}/./src/../.git/HEAD")), ".git/HEAD");
        assert_eq!(sub(&root_s), ".");
        let canonical = std::fs::canonicalize(&root)
            .expect("canonical")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(sub(&format!("{canonical}/src/a.rs")), "src/a.rs");
        let parent = root_s
            .rsplit_once('/')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default();
        assert_eq!(sub("../sibling/x.txt"), format!("{parent}/sibling/x.txt"));
        assert_eq!(sub("/etc/passwd"), "/etc/passwd");
        // A sibling directory sharing the root's name as a prefix is NOT inside the root.
        assert_eq!(sub(&format!("{root_s}2/x")), format!("{root_s}2/x"));
    }

    #[test]
    fn dotdot_cannot_widen_write_allowlist() {
        let cfg = dev_cfg();
        // `src/**` used to match these raw strings and auto-allow root-level writes.
        for p in [
            "src/../.github/workflows/ci.yml",
            "src/../build.rs",
            "src/../X",
        ] {
            let (v, reason) = write(&cfg, p);
            assert_eq!(v, PolicyVerdict::Deny, "{p}: {reason}");
        }
        // The normalized form is what gets reported.
        let (_, reason) = write(&cfg, "src/../build.rs");
        assert!(reason.contains("`build.rs`"), "{reason}");
        // A legitimate in-allowlist path still passes, also after normalization.
        assert_eq!(write(&cfg, "src/lib.rs").0, PolicyVerdict::Allow);
        assert_eq!(write(&cfg, "./src/lib.rs").0, PolicyVerdict::Allow);
        assert_eq!(write(&cfg, "tests/../src/lib.rs").0, PolicyVerdict::Allow);
    }

    #[test]
    fn dot_prefix_and_dotdot_cannot_evade_git_deny() {
        let cfg = dev_cfg();
        for p in [
            "./.git/config",
            "src/../.git/config",
            ".//.git/hooks/pre-commit",
        ] {
            let (wv, wr) = write(&cfg, p);
            assert_eq!(wv, PolicyVerdict::Deny, "write {p}: {wr}");
            assert!(wr.contains("deny rule `.git/**`"), "write {p}: {wr}");
            let (rv, rr) = read(&cfg, p);
            assert_eq!(rv, PolicyVerdict::Deny, "read {p}: {rr}");
            assert!(rr.contains("deny rule `.git/**`"), "read {p}: {rr}");
        }
    }

    #[test]
    fn absolute_paths_match_globs_and_unresolved_escapes_are_denied() {
        let cfg = dev_cfg(); // read.allow = ["**"]; write.allow = project-relative globs
        // Absolute paths are no longer hard-denied: a catch-all allow matches them ...
        for p in ["/etc/passwd", "/repo/src/lib.rs"] {
            let (rv, rr) = read(&cfg, p);
            assert_eq!(rv, PolicyVerdict::Allow, "read {p}: {rr}");
            // ... while project-relative write globs (`src/**`) do not.
            let (wv, wr) = write(&cfg, p);
            assert_eq!(wv, PolicyVerdict::Deny, "write {p}: {wr}");
        }
        // Deny globs still apply to absolute subjects.
        let (rv, rr) = read(&cfg, "/elsewhere/.env.local");
        assert_eq!(rv, PolicyVerdict::Deny, "{rr}");
        // A root-less `..` escape cannot be resolved and is denied.
        let (rv, rr) = read(&cfg, "../outside.txt");
        assert_eq!(rv, PolicyVerdict::Deny, "{rr}");
        assert!(rr.contains("above the project root"), "{rr}");
    }

    /// `--add-dir` style: a policy profile combined with an additional root outside the
    /// workspace. Paths into the extra root are evaluated as absolute subjects, so an explicit
    /// absolute glob (or `**`) can allow them while deny globs still bite.
    #[test]
    fn additional_root_paths_are_matchable_with_absolute_globs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repo");
        let extra = dir.path().join("shared");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::create_dir_all(&extra).expect("mkdir");
        let extra_s = extra.to_string_lossy().replace('\\', "/");
        let mut cfg = dev_cfg();
        cfg.filesystem.write.allow.push(format!("{extra_s}/**"));
        cfg.filesystem.write.deny.push("**/secrets/**".to_string());
        let eval_write = |p: &str| {
            let subject = policy_subject_path(std::path::Path::new(p), &root);
            cfg.evaluate_permission(&Permission::WriteFile { path: subject })
        };
        // Absolute path into the extra root.
        let (v, r) = eval_write(&format!("{extra_s}/notes.md"));
        assert_eq!(v, PolicyVerdict::Allow, "{r}");
        // Relative `../shared/...` resolves to the same absolute subject.
        let (v, r) = eval_write("../shared/notes.md");
        assert_eq!(v, PolicyVerdict::Allow, "{r}");
        // Deny globs still apply inside the extra root.
        let (v, r) = eval_write(&format!("{extra_s}/secrets/key.txt"));
        assert_eq!(v, PolicyVerdict::Deny, "{r}");
        // An absolute path inside the workspace is matched as its relative form.
        let root_s = root.to_string_lossy().replace('\\', "/");
        let (v, r) = eval_write(&format!("{root_s}/src/lib.rs"));
        assert_eq!(v, PolicyVerdict::Allow, "{r}");
        let (v, r) = eval_write(&format!("{root_s}/.git/hooks/pre-commit"));
        assert_eq!(v, PolicyVerdict::Deny, "{r}");
        // Without an explicit allow, other outside paths stay denied for writes.
        let (v, r) = eval_write("/etc/hosts");
        assert_eq!(v, PolicyVerdict::Deny, "{r}");
    }

    #[test]
    fn project_root_still_matches_catch_all_read() {
        let cfg = dev_cfg();
        assert_eq!(read(&cfg, ".").0, PolicyVerdict::Allow);
        let list = cfg.evaluate_permission(&Permission::ListDirectory {
            path: PathBuf::from("./src/.."),
        });
        assert_eq!(list.0, PolicyVerdict::Allow, "{}", list.1);
    }
}
