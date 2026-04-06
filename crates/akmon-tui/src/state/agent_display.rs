//! High-level agent activity shown in chrome and hints.

/// Coarse UI state for “what the agent is doing right now”.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AgentDisplayState {
    /// No turn in progress.
    #[default]
    Idle,
    /// Model is thinking (no tokens yet or between tool calls).
    Thinking,
    /// Executing a named tool.
    CallingTool {
        /// Tool name from the registry.
        tool_name: String,
        /// Monotonic step index from the session loop when known.
        step: u32,
    },
    /// Assistant stream in flight.
    Streaming {
        /// Characters received this turn (approximate).
        chars_received: u64,
    },
    /// Policy dialog is visible; input is disabled.
    WaitingForConfirmation,
    /// First request to the provider.
    Connecting {
        /// Human-readable provider label.
        provider: String,
    },
}
