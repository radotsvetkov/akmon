//! Pure transition validation for the v1 FSM table.

use super::error::AgentError;
use super::event::AgentEvent;
use super::state::AgentState;

/// Returns [`Ok(())`] when `(from, event)` is a **legal** v1 transition.
///
/// The **next** state is implied by the table (enforced by the executor in a later
/// slice). Illegal pairs return [`AgentError::InvalidTransition`].
///
/// # Legal transitions (v1)
///
/// - **Idle** + user input (`TextDelta`, or `IterationStarted` with `n == 1`) → *Planning*
/// - **Planning** + `TextDelta` → *Thinking* (model began responding)
/// - **Planning** / **Thinking** + `SummarizationStarted` → *Summarizing* (context compaction)
/// - **Planning** + `Error(model | policy | iteration limit)` → *Failed*
/// - **Thinking** + `ToolCallDispatched` → *ToolExecution*
/// - **Thinking** + `ConfirmationRequired` → *AwaitingConfirmation*
/// - **Thinking** + `Done` → *Complete*
/// - **Thinking** + `Error` (model / truncation / tool / policy / session / iteration) → *Failed*
/// - **ToolExecution** + `ToolCallCompleted` → *Thinking* or *Failed* (both event shapes allowed)
/// - **ToolExecution** + `Error` (tool / session / model) → *Failed*
/// - **AwaitingConfirmation** + `TextDelta` → *Thinking* (user confirmed or declined in-line)
/// - **AwaitingConfirmation** + `Error(SessionFailed)` → *Failed* (e.g. headless timeout)
/// - **Summarizing** + `ContextSummarized` → *Thinking*
/// - **Summarizing** + `Error(session | model)` → *Failed*
pub fn validate_transition(from: &AgentState, event: &AgentEvent) -> Result<(), AgentError> {
    let invalid = || AgentError::InvalidTransition {
        from: from.to_string(),
        to: event.to_string(),
    };

    match (from, event) {
        // Idle + user input → Planning
        (AgentState::Idle, AgentEvent::TextDelta { .. }) => Ok(()), // user typed / pasted
        (AgentState::Idle, AgentEvent::IterationStarted { n, .. }) if *n == 1 => Ok(()), // explicit turn kickoff

        // Planning + model stream → Thinking
        (AgentState::Planning { .. }, AgentEvent::TextDelta { .. }) => Ok(()), // model responded

        // Planning / Thinking / ToolExecution: token usage metadata (no state change)
        (AgentState::Planning { .. }, AgentEvent::UsageReport { .. }) => Ok(()),
        (AgentState::Thinking { .. }, AgentEvent::UsageReport { .. }) => Ok(()),
        (AgentState::ToolExecution { .. }, AgentEvent::UsageReport { .. }) => Ok(()),

        // Provider label from the live stream (no state change)
        (AgentState::Planning { .. }, AgentEvent::ProviderConfirmed { .. }) => Ok(()),
        (AgentState::Thinking { .. }, AgentEvent::ProviderConfirmed { .. }) => Ok(()),

        // Same states: status-only messages (no state change)
        (AgentState::Planning { .. }, AgentEvent::StatusInfo { .. }) => Ok(()),
        (AgentState::Thinking { .. }, AgentEvent::StatusInfo { .. }) => Ok(()),
        (AgentState::ToolExecution { .. }, AgentEvent::StatusInfo { .. }) => Ok(()),
        (AgentState::Summarizing { .. }, AgentEvent::StatusInfo { .. }) => Ok(()),

        // Microcompact estimate (token hygiene; no state change)
        (AgentState::Idle, AgentEvent::MicrocompactEstimate { .. }) => Ok(()),
        (AgentState::Planning { .. }, AgentEvent::MicrocompactEstimate { .. }) => Ok(()),
        (AgentState::Thinking { .. }, AgentEvent::MicrocompactEstimate { .. }) => Ok(()),
        (AgentState::ToolExecution { .. }, AgentEvent::MicrocompactEstimate { .. }) => Ok(()),
        (AgentState::Summarizing { .. }, AgentEvent::MicrocompactEstimate { .. }) => Ok(()),
        (
            AgentState::AwaitingConfirmation { .. },
            AgentEvent::MicrocompactEstimate { .. },
        ) => Ok(()),

        // Planning / Thinking → Summarizing (context compaction)
        (AgentState::Planning { .. }, AgentEvent::SummarizationStarted) => Ok(()),
        (AgentState::Thinking { .. }, AgentEvent::SummarizationStarted) => Ok(()),

        // Thinking + further stream chunks (same turn)
        (AgentState::Thinking { .. }, AgentEvent::TextDelta { .. }) => Ok(()),

        // Planning + terminal errors → Failed
        (
            AgentState::Planning { .. },
            AgentEvent::Error {
                error:
                    AgentError::ModelError { .. }
                    | AgentError::PolicyDenied { .. }
                    | AgentError::IterationLimitReached { .. },
                ..
            },
        ) => Ok(()),
        (AgentState::Planning { .. }, AgentEvent::Error { .. }) => Err(invalid()),

        // Thinking + tool path
        (AgentState::Thinking { .. }, AgentEvent::ToolCallDispatched { .. }) => Ok(()), // → ToolExecution

        // Thinking + tool outcome without dispatch (unknown tool, policy block, etc.)
        (AgentState::Thinking { .. }, AgentEvent::ToolCallCompleted { success: false, .. }) => {
            Ok(())
        }

        // Thinking + confirmation gate
        (AgentState::Thinking { .. }, AgentEvent::ConfirmationRequired { .. }) => {
            Ok(()) // → AwaitingConfirmation
        }

        // Thinking + normal completion
        (AgentState::Thinking { .. }, AgentEvent::Done) => Ok(()), // → Complete

        // Thinking + failure (any structured error except nested invalid-transition meta-errors)
        (AgentState::Thinking { .. }, AgentEvent::Error { error, .. }) => {
            if matches!(error, AgentError::InvalidTransition { .. }) {
                Err(invalid())
            } else {
                Ok(())
            }
        }

        // ToolExecution → Thinking or Failed (tool finished or errored)
        (AgentState::ToolExecution { .. }, AgentEvent::ToolCallCompleted { .. }) => Ok(()), // success or failure both legal events

        // ask_followup: UI prompt before the tool row is finalized
        (AgentState::ToolExecution { .. }, AgentEvent::QuestionRequired { .. }) => Ok(()),

        // Additional dispatches in the same parallel tool batch (session tracks outstanding completions)
        (AgentState::ToolExecution { .. }, AgentEvent::ToolCallDispatched { .. }) => Ok(()),

        (
            AgentState::ToolExecution { .. },
            AgentEvent::Error {
                error:
                    AgentError::ToolError { .. }
                    | AgentError::SessionFailed { .. }
                    | AgentError::ModelError { .. },
                ..
            },
        ) => Ok(()),
        (AgentState::ToolExecution { .. }, AgentEvent::Error { .. }) => Err(invalid()),

        // AwaitingConfirmation → Thinking (user message) or Failed (timeout)
        (AgentState::AwaitingConfirmation { .. }, AgentEvent::TextDelta { .. }) => Ok(()), // → Thinking

        (
            AgentState::AwaitingConfirmation { .. },
            AgentEvent::Error {
                error: AgentError::SessionFailed { .. },
                ..
            },
        ) => Ok(()), // e.g. confirmation timeout in headless
        (AgentState::AwaitingConfirmation { .. }, AgentEvent::Error { .. }) => Err(invalid()),

        // Summarizing → Thinking or Failed
        (AgentState::Summarizing { .. }, AgentEvent::UsageReport { .. }) => Ok(()),
        (AgentState::Summarizing { .. }, AgentEvent::ContextSummarized { .. }) => Ok(()), // → Thinking

        (
            AgentState::Summarizing { .. },
            AgentEvent::Error {
                error: AgentError::SessionFailed { .. } | AgentError::ModelError { .. },
                ..
            },
        ) => Ok(()),
        (AgentState::Summarizing { .. }, AgentEvent::Error { .. }) => Err(invalid()),

        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn thinking(i: u32) -> AgentState {
        AgentState::Thinking { iteration: i }
    }

    fn planning() -> AgentState {
        AgentState::Planning {
            task: "t".into(),
            iteration: 0,
        }
    }

    #[test]
    fn legal_idle_text_to_planning() {
        assert!(
            validate_transition(
                &AgentState::Idle,
                &AgentEvent::TextDelta { text: "hi".into() }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_idle_iteration_one() {
        assert!(
            validate_transition(
                &AgentState::Idle,
                &AgentEvent::IterationStarted { n: 1, max: 25 }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_planning_text_to_thinking() {
        assert!(
            validate_transition(
                &planning(),
                &AgentEvent::TextDelta {
                    text: "model".into(),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_thinking_streaming_text_deltas() {
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::TextDelta {
                    text: "chunk".into(),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_thinking_status_info_no_state_change() {
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::StatusInfo {
                    message: "continuing…".into(),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_thinking_tool_completed_without_dispatch() {
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::ToolCallCompleted {
                    id: "1".into(),
                    name: "nope".into(),
                    success: false,
                    message: "tool not found".into(),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_planning_model_error_to_failed() {
        assert!(
            validate_transition(
                &planning(),
                &AgentEvent::Error {
                    error: AgentError::ModelError {
                        message: "x".into(),
                    },
                    recoverable: false,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_planning_policy_denied_to_failed() {
        assert!(
            validate_transition(
                &planning(),
                &AgentEvent::Error {
                    error: AgentError::PolicyDenied {
                        permission: "WriteFile".into(),
                        reason: "sandbox".into(),
                    },
                    recoverable: false,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_planning_iteration_limit_to_failed() {
        assert!(
            validate_transition(
                &planning(),
                &AgentEvent::Error {
                    error: AgentError::IterationLimitReached { limit: 25 },
                    recoverable: false,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_thinking_tool_dispatched() {
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::ToolCallDispatched {
                    id: "1".into(),
                    name: "read".into(),
                    arguments: json!({}),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_thinking_confirmation_required() {
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::ConfirmationRequired {
                    description: "rm".into(),
                    diff_preview: None,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_thinking_done_complete() {
        assert!(validate_transition(&thinking(0), &AgentEvent::Done).is_ok());
    }

    #[test]
    fn legal_thinking_truncated_failed() {
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::Error {
                    error: AgentError::ResponseTruncated,
                    recoverable: false,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_tool_execution_completed() {
        assert!(
            validate_transition(
                &AgentState::ToolExecution { iteration: 0 },
                &AgentEvent::ToolCallCompleted {
                    id: "1".into(),
                    name: "read".into(),
                    success: true,
                    message: String::new(),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_tool_execution_second_dispatched_stays_in_tool_execution() {
        assert!(
            validate_transition(
                &AgentState::ToolExecution { iteration: 0 },
                &AgentEvent::ToolCallDispatched {
                    id: "2".into(),
                    name: "read".into(),
                    arguments: json!({}),
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_tool_execution_tool_error() {
        assert!(
            validate_transition(
                &AgentState::ToolExecution { iteration: 0 },
                &AgentEvent::Error {
                    error: AgentError::ToolError {
                        tool: "bash".into(),
                        message: "fail".into(),
                    },
                    recoverable: true,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_awaiting_confirmation_text() {
        assert!(
            validate_transition(
                &AgentState::AwaitingConfirmation { iteration: 0 },
                &AgentEvent::TextDelta { text: "yes".into() }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_awaiting_confirmation_timeout_failed() {
        assert!(
            validate_transition(
                &AgentState::AwaitingConfirmation { iteration: 0 },
                &AgentEvent::Error {
                    error: AgentError::SessionFailed {
                        message: "confirmation timeout".into(),
                    },
                    recoverable: false,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_awaiting_confirmation_microcompact_estimate() {
        assert!(
            validate_transition(
                &AgentState::AwaitingConfirmation { iteration: 0 },
                &AgentEvent::MicrocompactEstimate {
                    estimated_tokens_cleared: 100,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_planning_usage_report() {
        assert!(
            validate_transition(
                &planning(),
                &AgentEvent::UsageReport {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_planning_summarization_started_to_summarizing() {
        assert!(validate_transition(&planning(), &AgentEvent::SummarizationStarted,).is_ok());
    }

    #[test]
    fn legal_thinking_summarization_started_to_summarizing() {
        assert!(validate_transition(&thinking(0), &AgentEvent::SummarizationStarted,).is_ok());
    }

    #[test]
    fn legal_summarizing_done_to_thinking() {
        assert!(
            validate_transition(
                &AgentState::Summarizing { iteration: 1 },
                &AgentEvent::ContextSummarized {
                    messages_replaced: 10,
                    tokens_freed: 1000,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn legal_summarizing_model_error_failed() {
        assert!(
            validate_transition(
                &AgentState::Summarizing { iteration: 1 },
                &AgentEvent::Error {
                    error: AgentError::ModelError {
                        message: "summary failed".into(),
                    },
                    recoverable: false,
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn illegal_five_cases() {
        assert!(validate_transition(&AgentState::Complete, &AgentEvent::Done).is_err());
        assert!(
            validate_transition(
                &AgentState::Idle,
                &AgentEvent::ToolCallDispatched {
                    id: "1".into(),
                    name: "x".into(),
                    arguments: json!({}),
                }
            )
            .is_err()
        );
        assert!(validate_transition(&planning(), &AgentEvent::Done).is_err());
        assert!(
            validate_transition(
                &thinking(0),
                &AgentEvent::ContextSummarized {
                    messages_replaced: 0,
                    tokens_freed: 0,
                }
            )
            .is_err()
        );
        assert!(
            validate_transition(
                &AgentState::ToolExecution { iteration: 0 },
                &AgentEvent::ConfirmationRequired {
                    description: "x".into(),
                    diff_preview: None,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn illegal_thinking_nested_invalid_transition_error() {
        let r = validate_transition(
            &thinking(0),
            &AgentEvent::Error {
                error: AgentError::InvalidTransition {
                    from: "x".into(),
                    to: "y".into(),
                },
                recoverable: false,
            },
        );
        assert!(r.is_err());
    }

    #[test]
    fn illegal_idle_iteration_not_one() {
        assert!(
            validate_transition(
                &AgentState::Idle,
                &AgentEvent::IterationStarted { n: 2, max: 25 }
            )
            .is_err()
        );
    }
}
