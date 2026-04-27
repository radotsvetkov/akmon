//! Session-scoped configuration for the agent FSM.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cost_estimate::ModelCostEstimateRow;

/// Tunables for iteration ceiling, confirmation timeouts, and session identity.
///
/// Values are read by the orchestrator (future slice); this module only holds data.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// When `true`, after each successful `edit` or `write_file`, run `git add` + `git commit` in the sandbox root.
    pub auto_commit: bool,
    /// When set, overrides the default completion `max_tokens` for this session (subagents, tests).
    pub max_completion_tokens: Option<u32>,
    /// Use compact nested-agent prompts (`spawn_subagent`) instead of the full project context block.
    pub subagent_style: bool,
    /// When set in headless mode, stop after estimated cumulative spend reaches this USD amount (ignored for free local models).
    pub max_budget_usd: Option<f64>,
    /// When the cloud API returns repeated rate limits, the last retries may switch to this model id (headless / completion layer).
    pub fallback_model: Option<String>,
    /// Optional per-model context/cost hints copied from `~/.akmon/config.toml` `[model_estimates]`.
    pub model_estimates: Vec<ModelCostEstimateRow>,
}

impl Default for AgentConfig {
    /// Builds the stock v1 defaults: **25** iterations, **30** seconds confirmation timeout,
    /// and a fresh random [`Uuid`] for [`AgentConfig::session_id`].
    fn default() -> Self {
        Self {
            max_iterations: 25,
            confirmation_timeout_secs: 30,
            session_id: Uuid::new_v4(),
            auto_commit: false,
            max_completion_tokens: None,
            subagent_style: false,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
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
        assert!(c.max_completion_tokens.is_none());
        assert!(!c.subagent_style);
    }
}
