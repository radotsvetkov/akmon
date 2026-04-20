//! Lightweight per-run reliability metrics for CI/ops observability.

use serde::{Deserialize, Serialize};

/// Aggregated reliability counters for one agent run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunReliabilityMetrics {
    /// Total tool calls completed (success + failure).
    pub tool_calls_total: u64,
    /// Tool calls that completed successfully.
    pub tool_calls_success: u64,
    /// Tool calls that completed with failure.
    pub tool_calls_failure: u64,
    /// Sum of tool call latencies in milliseconds.
    pub tool_latency_ms_total: u64,
    /// Integer average tool latency in milliseconds.
    pub tool_latency_ms_avg: u64,
    /// Optional p95 tool latency in milliseconds.
    pub tool_latency_ms_p95: Option<u64>,
    /// Number of policy denials observed during tool dispatch.
    pub policy_denials_total: u64,
    /// Number of automatic retry/continuation attempts observed by the session loop.
    pub retries_total: u64,
    /// Number of timeout outcomes observed by the session loop.
    pub timeouts_total: u64,
}
