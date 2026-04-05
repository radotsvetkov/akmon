//! Agent-level errors surfaced by the FSM and tool loop.

use thiserror::Error;

/// Recoverable or fatal failure while the agent runs.
///
/// Each variant maps to user-visible messaging and audit records. Implements
/// [`std::fmt::Display`] via [`thiserror::Error`] for stable `to_string()` output.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentError {
    /// The hard iteration ceiling was hit; the loop must stop.
    #[error("iteration limit reached (limit={limit})")]
    IterationLimitReached {
        /// Configured maximum iterations (inclusive bound in [`super::check_iteration_limit`]).
        limit: u32,
    },
    /// The model provider returned an error or unusable response.
    #[error("model error: {message}")]
    ModelError {
        /// Provider or transport message (must not contain secrets).
        message: String,
    },
    /// A tool invocation failed after dispatch.
    #[error("tool '{tool}' failed: {message}")]
    ToolError {
        /// Tool name as registered with Akmon.
        tool: String,
        /// Failure description (no secrets).
        message: String,
    },
    /// The policy engine denied a permission request.
    #[error("policy denied ({permission}): {reason}")]
    PolicyDenied {
        /// Permission identifier or summary (e.g. tool name + action).
        permission: String,
        /// Policy explanation.
        reason: String,
    },
    /// Model output was cut off before a complete structured response.
    #[error("model response truncated")]
    ResponseTruncated,
    /// Session-level failure (e.g. I/O, handshake, confirmation timeout in headless mode).
    #[error("session failed: {message}")]
    SessionFailed {
        /// Human-readable cause (no secrets).
        message: String,
    },
    /// Proposed FSM `(state, event)` pair is not allowed in v1.
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition {
        /// [`super::AgentState`] description before the transition.
        from: String,
        /// [`super::AgentEvent`] or target description for the attempted step.
        to: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iteration_limit_reached_display() {
        let e = AgentError::IterationLimitReached { limit: 25 };
        assert!(e.to_string().contains("25"));
    }

    #[test]
    fn model_error_display() {
        let e = AgentError::ModelError {
            message: "unavailable".into(),
        };
        assert!(e.to_string().contains("unavailable"));
    }

    #[test]
    fn tool_error_display() {
        let e = AgentError::ToolError {
            tool: "bash".into(),
            message: "exit 1".into(),
        };
        let s = e.to_string();
        assert!(s.contains("bash") && s.contains("exit 1"));
    }

    #[test]
    fn policy_denied_display() {
        let e = AgentError::PolicyDenied {
            permission: "NetworkFetch".into(),
            reason: "host not allowlisted".into(),
        };
        let s = e.to_string();
        assert!(s.contains("NetworkFetch") && s.contains("allowlisted"));
    }

    #[test]
    fn response_truncated_display() {
        let e = AgentError::ResponseTruncated;
        assert!(!e.to_string().is_empty());
    }

    #[test]
    fn session_failed_display() {
        let e = AgentError::SessionFailed {
            message: "confirmation timeout".into(),
        };
        assert!(e.to_string().contains("timeout"));
    }

    #[test]
    fn invalid_transition_display() {
        let e = AgentError::InvalidTransition {
            from: "Idle".into(),
            to: "ToolCallDispatched".into(),
        };
        let s = e.to_string();
        assert!(s.contains("Idle") && s.contains("ToolCallDispatched"));
    }
}
