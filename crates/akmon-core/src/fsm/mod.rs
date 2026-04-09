//! Finite-state machine types for the agent session (data and validation only).
//!
//! Execution, async I/O, model calls, and tool dispatch are intentionally **not**
//! implemented in this module — only [`AgentState`], [`AgentEvent`], [`AgentError`],
//! [`AgentConfig`], [`validate_transition`], and [`check_iteration_limit`].

mod config;
mod error;
mod event;
mod state;
mod transition;

pub use config::AgentConfig;
pub use error::AgentError;
pub use event::AgentEvent;
pub use state::AgentState;
pub use transition::validate_transition;

/// Enforces the configured iteration ceiling before advancing the loop.
///
/// Returns [`Err(AgentError::IterationLimitReached)`] when `iteration >= config.max_iterations`.
/// When the limit is reached, the orchestrator must stop the agent loop and surface the error
/// to the user (no silent retry).
///
/// For `iteration` values **strictly less than** `max_iterations`, returns [`Ok(())`].
pub fn check_iteration_limit(iteration: u32, config: &AgentConfig) -> Result<(), AgentError> {
    if iteration >= config.max_iterations {
        return Err(AgentError::IterationLimitReached {
            limit: config.max_iterations,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn check_iteration_limit_ok_below_max() {
        let config = AgentConfig {
            max_iterations: 25,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
            auto_commit: false,
            max_completion_tokens: None,
            subagent_style: false,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
        };
        assert!(check_iteration_limit(0, &config).is_ok());
        assert!(check_iteration_limit(24, &config).is_ok());
    }

    #[test]
    fn check_iteration_limit_err_at_max() {
        let config = AgentConfig {
            max_iterations: 25,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
            auto_commit: false,
            max_completion_tokens: None,
            subagent_style: false,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
        };
        assert_eq!(
            check_iteration_limit(25, &config),
            Err(AgentError::IterationLimitReached { limit: 25 })
        );
    }

    #[test]
    fn check_iteration_limit_err_above_max() {
        let config = AgentConfig {
            max_iterations: 10,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
            auto_commit: false,
            max_completion_tokens: None,
            subagent_style: false,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
        };
        assert!(check_iteration_limit(100, &config).is_err());
    }
}
