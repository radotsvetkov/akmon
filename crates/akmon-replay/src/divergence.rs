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
