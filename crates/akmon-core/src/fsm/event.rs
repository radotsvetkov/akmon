//! Events that drive FSM transitions (streaming deltas, tools, confirmations, …).

use std::fmt;

use super::error::AgentError;

/// Observable occurrence processed by the agent loop when deciding the next state.
///
/// Carries payloads needed for audit logs and UI; this slice does not execute handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// Incremental natural-language text (user or model stream chunk).
    TextDelta {
        /// UTF-8 fragment; may be empty only when used as a protocol keep-alive (illegal for some transitions).
        text: String,
    },
    /// A tool invocation was sent to the execution layer.
    ToolCallDispatched {
        /// Stable tool-use id from the model or runtime.
        id: String,
        /// Registered tool name.
        name: String,
    },
    /// A tool invocation finished (success or failure).
    ToolCallCompleted {
        /// Same id as in [`AgentEvent::ToolCallDispatched`].
        id: String,
        /// Tool name.
        name: String,
        /// When `false`, the transition targets [`super::AgentState::Failed`] after this event.
        success: bool,
        /// Human-readable outcome (error text when `success` is `false`, optional detail when `true`).
        message: String,
    },
    /// User must confirm before a sensitive or destructive step proceeds.
    ConfirmationRequired {
        /// Short description shown in the transparency strip / headless error text.
        description: String,
    },
    /// Context summarization has started (history compaction before the summary model call).
    SummarizationStarted,
    /// Context summarization replaced prior messages with a compact summary.
    ContextSummarized {
        /// Number of messages removed or folded into the summary.
        messages_replaced: usize,
        /// Estimated tokens reclaimed (best-effort).
        tokens_freed: usize,
    },
    /// Marks the start of iteration `n` of at most `max` (inclusive ceiling check uses `max`).
    IterationStarted {
        /// 1-based or 0-based per orchestrator convention; `n == 1` often means a new user turn from [`super::AgentState::Idle`].
        n: u32,
        /// Same value as [`super::AgentConfig::max_iterations`] when emitted by the runtime.
        max: u32,
    },
    /// The model signalled end-of-turn with no further tool work.
    Done,
    /// One completion’s token usage (e.g. Anthropic input, output, and prompt-cache counters).
    UsageReport {
        /// Input tokens billed for this model request.
        input_tokens: u32,
        /// Output tokens generated in this completion.
        output_tokens: u32,
        /// Tokens charged for creating prompt-cache entries.
        cache_creation_tokens: u32,
        /// Tokens read from the prompt cache (non-zero when the cache was used).
        cache_read_tokens: u32,
    },
    /// A structured failure or policy outcome wrapped as an event.
    Error {
        /// Failure classification.
        error: AgentError,
        /// Whether the session may recover (mirrors [`super::AgentState::Failed`]).
        recoverable: bool,
    },
}

impl fmt::Display for AgentEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentEvent::TextDelta { .. } => write!(f, "TextDelta"),
            AgentEvent::ToolCallDispatched { name, .. } => {
                write!(f, "ToolCallDispatched({name})")
            }
            AgentEvent::ToolCallCompleted {
                name, success, message, ..
            } => write!(
                f,
                "ToolCallCompleted({name}, success={success}, message={message})"
            ),
            AgentEvent::ConfirmationRequired { .. } => write!(f, "ConfirmationRequired"),
            AgentEvent::SummarizationStarted => write!(f, "SummarizationStarted"),
            AgentEvent::ContextSummarized { .. } => write!(f, "ContextSummarized"),
            AgentEvent::IterationStarted { n, max } => {
                write!(f, "IterationStarted(n={n}, max={max})")
            }
            AgentEvent::Done => write!(f, "Done"),
            AgentEvent::UsageReport {
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                ..
            } => write!(
                f,
                "UsageReport(input={input_tokens}, cache_read={cache_read_tokens}, cache_write={cache_creation_tokens})"
            ),
            AgentEvent::Error { error, .. } => write!(f, "Error({error})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_tool_dispatched() {
        let e = AgentEvent::ToolCallDispatched {
            id: "1".into(),
            name: "read".into(),
        };
        assert!(e.to_string().contains("read"));
    }
}
