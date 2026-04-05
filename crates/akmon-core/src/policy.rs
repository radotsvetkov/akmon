//! Policy engine: deny-all default, interactive resolution, and configured rules.

use chrono::Utc;
use thiserror::Error;

use crate::audit::{AuditEvent, PolicyVerdict};
use crate::permission::Permission;

/// Rule set loaded from project configuration (fields added in a later slice).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyConfig {
    // Intentionally empty — declarative rules land in the next policy slice.
}

impl PolicyConfig {
    /// Applies configured rules to `permission`.
    ///
    /// Skeleton: no rules are defined yet; every request is denied with a
    /// stable explanation until rule evaluation is implemented.
    pub fn evaluate_permission(&self, _permission: &Permission) -> (PolicyVerdict, String) {
        (
            PolicyVerdict::Deny,
            "denied: PolicyConfig defines no matching rules yet (skeleton)".to_string(),
        )
    }
}

/// How the engine decides automatic permissions (anything other than a live
/// caller verdict in interactive mode).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// Rules from [`PolicyConfig`] (skeleton denies until rules exist).
    Configured(PolicyConfig),
}

/// Error returned when the engine is used in a way that does not match its mode.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyEngineError {
    /// [`PolicyEngineMode::Interactive`] (or write confirmation under [`PolicyEngineMode::AutoApproveReads`]
    /// or [`PolicyEngineMode::AutoApproveReadsAndFetch`]) does not support automatic resolution for this permission.
    #[error("interactive confirmation required; use resolve_interactive after the user supplies a verdict")]
    InteractiveRequiresCaller,
    /// [`PolicyEngine::resolve_interactive`] was called while not in interactive or auto-read-approve mode.
    #[error("resolve_interactive is only valid in Interactive, AutoApproveReads, or AutoApproveReadsAndFetch mode")]
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
    /// and the allow branch of [`PolicyEngineMode::Interactive`]
    /// (which always signals that the caller must prompt).
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
            PolicyEngineMode::AutoApproveReads { confirm_writes } => {
                match &permission {
                    Permission::ReadFile { .. } | Permission::ListDirectory { .. } => {
                        (true, "auto-approved read (--yes)".to_string(), PolicyVerdict::Allow)
                    }
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
                }
            }
            PolicyEngineMode::AutoApproveReadsAndFetch { confirm_writes } => {
                match &permission {
                    Permission::ReadFile { .. } | Permission::ListDirectory { .. } => {
                        (true, "auto-approved read (--yes)".to_string(), PolicyVerdict::Allow)
                    }
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
                }
            }
            PolicyEngineMode::Configured(cfg) => {
                let (verdict, r) = cfg.evaluate_permission(&permission);
                let allowed = matches!(verdict, PolicyVerdict::Allow);
                (allowed, r, verdict)
            }
        };

        Ok(Self::decision_from_parts(
            session_id,
            timestamp,
            permission,
            verdict,
            allowed,
            reason,
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
            session_id,
            timestamp,
            permission,
            verdict,
            allowed,
            reason,
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
    fn configured_skeleton_denies() {
        let engine = PolicyEngine::new(PolicyEngineMode::Configured(PolicyConfig::default()));
        let perm = Permission::NetworkFetch {
            url: "https://example.com".into(),
        };
        let decision = engine.evaluate_automatic("sess-c", perm).unwrap();
        assert!(!decision.allowed);
        assert!(decision.audit.to_json().expect("audit json").contains("deny"));
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
            .resolve_interactive(
                "sess-i",
                perm,
                PolicyVerdict::Allow,
                "user approved in TUI",
            )
            .unwrap();
        assert!(decision.allowed);
        assert!(decision.audit.to_json().expect("audit json").contains("allow"));
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
