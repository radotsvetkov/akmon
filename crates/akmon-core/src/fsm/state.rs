//! Discrete agent states for the session FSM.

use std::fmt;

use super::error::AgentError;

/// High-level phase of a single agent session.
///
/// States are **data-only** in this slice; execution logic assigns transitions later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    /// Session is waiting for the next user message or external kick.
    ///
    /// Entered at session start and after a turn fully completes (`Complete`) if the
    /// session stays open.
    Idle,
    /// User input was accepted and the agent is forming or tracking a concrete task plan.
    ///
    /// Entered from [`super::AgentState::Idle`] when user text or an initial iteration
    /// marker arrives. `task` is a short description of the current objective; `iteration`
    /// is the active loop counter for this planning episode.
    Planning {
        /// Human-readable task label or summary (not the full prompt).
        task: String,
        /// Current iteration index for this phase (ties to the global iteration ceiling).
        iteration: u32,
    },
    /// The model is producing reasoning or natural-language output for the current turn.
    ///
    /// Entered after planning when the backend begins streaming or a complete model reply
    /// is being processed.
    Thinking {
        /// Current iteration index (must stay below [`super::AgentConfig::max_iterations`]).
        iteration: u32,
    },
    /// One or more tool calls are in flight for this turn.
    ///
    /// Entered from [`super::AgentState::Thinking`] when the model requests tools.
    ToolExecution {
        /// Iteration index associated with this tool batch.
        iteration: u32,
    },
    /// A destructive or sensitive action needs explicit user confirmation.
    ///
    /// Entered from [`super::AgentState::Thinking`] when policy requires approval before
    /// proceeding (interactive TUI or headless pre-approval).
    AwaitingConfirmation {
        /// Iteration index for this confirmation gate.
        iteration: u32,
    },
    /// Context compaction or summarization is running to free tokens.
    ///
    /// Entered when the orchestrator decides to summarize prior messages (exact entry
    /// events are defined in the execution slice). `iteration` ties the summarize step
    /// to the global loop counter.
    Summarizing {
        /// Iteration index during summarization.
        iteration: u32,
    },
    /// The turn finished successfully with a final answer and no pending work.
    ///
    /// Terminal success state until the user starts a new turn (returning to `Idle`).
    Complete,
    /// The turn or session aborted with an error.
    ///
    /// `recoverable` hints whether the UI may offer retry; `error` carries the cause.
    Failed {
        /// Structured failure reason.
        error: AgentError,
        /// If true, the session may continue after user action; if false, stop or reset.
        recoverable: bool,
    },
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentState::Idle => write!(f, "Idle"),
            AgentState::Planning { task, iteration } => {
                write!(f, "Planning(task={task}, iteration={iteration})")
            }
            AgentState::Thinking { iteration } => write!(f, "Thinking(iteration={iteration})"),
            AgentState::ToolExecution { iteration } => {
                write!(f, "ToolExecution(iteration={iteration})")
            }
            AgentState::AwaitingConfirmation { iteration } => {
                write!(f, "AwaitingConfirmation(iteration={iteration})")
            }
            AgentState::Summarizing { iteration } => {
                write!(f, "Summarizing(iteration={iteration})")
            }
            AgentState::Complete => write!(f, "Complete"),
            AgentState::Failed { .. } => write!(f, "Failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_idle() {
        assert_eq!(AgentState::Idle.to_string(), "Idle");
    }

    #[test]
    fn display_planning_includes_task() {
        let s = AgentState::Planning {
            task: "fix bug".into(),
            iteration: 2,
        }
        .to_string();
        assert!(s.contains("fix bug") && s.contains('2'));
    }
}
