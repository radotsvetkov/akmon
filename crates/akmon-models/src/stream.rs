//! Streaming completion events and stop reasons.

use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::ModelError;

/// One tool invocation requested by the model (OpenAI-style `tool_calls` entry, normalized).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelToolCall {
    /// Stable id from the provider (e.g. `call_abc`).
    pub id: String,
    /// Registered tool name (e.g. `read_file`).
    pub name: String,
    /// Parsed JSON arguments object (empty object if the model sent none).
    pub arguments: serde_json::Value,
}

/// Token accounting for one model completion, including Anthropic prompt-cache fields when present.
///
/// [`StreamEvent::UsageReport`] carries this after the provider merges `message_start` usage with
/// final fields from streaming `message_delta` events when present (`output_tokens`, `input_tokens`,
/// and prompt-cache token counts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageReport {
    /// Billable input tokens for this request (Anthropic `input_tokens`).
    pub input_tokens: u32,
    /// Generated output tokens for this completion (Anthropic `output_tokens`).
    pub output_tokens: u32,
    /// Tokens written to the ephemeral prompt cache (`cache_creation_input_tokens`).
    pub cache_creation_tokens: u32,
    /// Tokens read from the ephemeral prompt cache (`cache_read_input_tokens`); non-zero implies a cache hit.
    pub cache_read_tokens: u32,
}

/// Why the model stopped generating for this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Normal end of assistant text for the turn — the agent FSM may treat the turn as complete
    /// (return toward idle / wait for the next user message) unless higher-level policy says otherwise.
    EndTurn,
    /// Generation stopped because [`crate::CompletionConfig::max_tokens`] was hit — the loop should
    /// surface this to the user and avoid silent continuation without a new prompt or config.
    MaxTokens,
    /// The model emitted tool calls — the agent loop must dispatch tools, append results, and
    /// typically re-enter a model call rather than finishing the session.
    ToolUse,
}

/// One item from a streaming completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// Incremental assistant text.
    TextDelta {
        /// UTF-8 fragment (may be empty in edge cases; callers should concatenate).
        text: String,
    },
    /// The model has finished this completion.
    Done {
        /// Explains what the orchestrator should do next.
        stop_reason: StopReason,
        /// Tool invocations when `stop_reason` is [`StopReason::ToolUse`]; otherwise empty.
        tool_calls: Vec<ModelToolCall>,
    },
    /// Emitted after the provider has assembled token usage for this completion (once per HTTP stream).
    ///
    /// Sent immediately before [`StreamEvent::Done`] when the backend supplies usage (e.g. Anthropic).
    /// [`UsageReport::cache_read_tokens`] greater than zero indicates a prompt-cache hit.
    UsageReport(UsageReport),
    /// A fatal or recoverable error surfaced on the stream after chunks may have been sent.
    Error {
        /// Structured failure.
        error: ModelError,
    },
}

/// Boxed, pinned, [`Send`] stream of completion results suitable for async executors.
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ModelError>> + Send>>;

#[cfg(test)]
mod usage_report_tests {
    use super::UsageReport;

    #[test]
    fn usage_report_serializes_to_expected_json() {
        let u = UsageReport {
            input_tokens: 100,
            output_tokens: 20,
            cache_creation_tokens: 50,
            cache_read_tokens: 200,
        };
        let v = serde_json::to_value(&u).expect("serialize");
        assert_eq!(v["input_tokens"], 100);
        assert_eq!(v["output_tokens"], 20);
        assert_eq!(v["cache_creation_tokens"], 50);
        assert_eq!(v["cache_read_tokens"], 200);
    }
}
