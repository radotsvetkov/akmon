use std::sync::{Arc, Mutex};

/// Divergence categories recorded during replay runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayDivergenceKind {
    /// Provider was called where source history did not expect a provider call.
    ProviderCallUnexpected,
    /// Provider request hash differs from source history.
    ProviderRequestMismatch,
    /// Strict mode attempt counts differ.
    AttemptCountDivergence,
    /// Strict mode attempt statuses differ.
    AttemptStatusDivergence,
    /// Tool was called where source history did not expect a tool call.
    ToolCallUnexpected,
    /// Tool input hash differs from source history.
    ToolInputMismatch,
    /// Session ended unexpectedly relative to source ordering.
    UnexpectedSessionEnd,
    /// Source history has expected events replay did not reach.
    MissingExpectedEvent,
    /// Source and replay event kinds differ at the same index.
    EventKindMismatch,
    /// Source history contains events replay history is missing.
    MissingReplayEvent,
    /// Replay history contains extra events not present in source.
    UnexpectedReplayEvent,
    /// Source and replay event counts differ.
    EventCountMismatch,
    /// Assistant content differs for matching `AssistantTurn` events.
    AssistantContentMismatch,
    /// Tool output differs for matching `ToolCall` events.
    ToolOutputMismatch,
    /// Permission decision differs for matching `PermissionGate` events.
    PermissionGateDecisionMismatch,
    /// Generic hash/reference mismatch for a specific field within matching event kinds.
    ContentReferenceMismatch,
}

/// One replay divergence record with human-readable expected/actual summaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayDivergence {
    /// Source event sequence where divergence occurred (when known).
    pub event_seq: Option<u64>,
    /// Divergence category.
    pub kind: ReplayDivergenceKind,
    /// Expected behavior summary.
    pub expected: String,
    /// Observed behavior summary.
    pub actual: String,
}

/// Shared divergence collector used by playback primitives and engine.
pub type ReplayDivergenceCollector = Arc<Mutex<Vec<ReplayDivergence>>>;
