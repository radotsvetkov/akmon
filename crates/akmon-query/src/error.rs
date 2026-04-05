//! Errors used by the query layer (provider, policy, tools, FSM wiring).

use akmon_core::AgentError;
use akmon_models::ModelError;
use thiserror::Error;

/// Failure while running an [`crate::AgentSession`] or its dependencies.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Raised when the configured [`akmon_models::LlmProvider`] returns [`ModelError`].
    #[error("provider error: {0}")]
    ProviderError(#[from] ModelError),
    /// Raised when policy denies or misconfigures an action (human-readable `String` only).
    #[error("policy error: {0}")]
    PolicyError(String),
    /// Raised when a tool fails or cannot be invoked as requested.
    #[error("tool error: {0}")]
    ToolError(String),
    /// Wraps [`AgentError`] from the core FSM (iteration limit, invalid transition, etc.).
    #[error("state machine error: {0}")]
    StateMachineError(#[from] AgentError),
}
