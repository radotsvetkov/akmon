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
    /// A fatal or recoverable error surfaced on the stream after chunks may have been sent.
    Error {
        /// Structured failure.
        error: ModelError,
    },
}

/// Boxed, pinned, [`Send`] stream of completion results suitable for async executors.
pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<StreamEvent, ModelError>> + Send>>;
