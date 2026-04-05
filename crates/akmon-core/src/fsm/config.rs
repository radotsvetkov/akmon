//! Session-scoped configuration for the agent FSM.

use uuid::Uuid;

/// Tunables for iteration ceiling, confirmation timeouts, and session identity.
///
/// Values are read by the orchestrator (future slice); this module only holds data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    /// Maximum number of iterations per session turn before [`super::check_iteration_limit`] fails with [`super::AgentError::IterationLimitReached`].
    ///
    /// Must be **at least 1**. When the running `iteration` index is **greater than or
    /// equal** to this value, the loop must stop and surface `IterationLimitReached`
    /// (hard ceiling — no silent continuation).
    pub max_iterations: u32,
    /// Seconds to wait for user confirmation when policy requires approval.
    ///
    /// In **headless** mode with no TTY, exceeding this timeout must fail the turn with
    /// [`super::AgentError::SessionFailed`] (e.g. message `confirmation timeout`) rather
    /// than blocking indefinitely.
    pub confirmation_timeout_secs: u64,
    /// Stable id for audit records and telemetry correlation for this session.
    pub session_id: Uuid,
}

impl Default for AgentConfig {
    /// Builds the stock v1 defaults: **25** iterations, **30** seconds confirmation timeout,
    /// and a fresh random [`Uuid`] for [`AgentConfig::session_id`].
    fn default() -> Self {
        Self {
            max_iterations: 25,
            confirmation_timeout_secs: 30,
            session_id: Uuid::new_v4(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_iterations_and_timeout() {
        let c = AgentConfig::default();
        assert_eq!(c.max_iterations, 25);
        assert_eq!(c.confirmation_timeout_secs, 30);
    }
}
