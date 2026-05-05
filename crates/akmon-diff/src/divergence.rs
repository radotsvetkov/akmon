use serde::{Deserialize, Serialize};

/// Divergence categories emitted by diff comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffDivergenceKind {
    /// Session event counts differ.
    EventCountMismatch,
    /// Event kinds differ at the same lockstep position.
    EventKindMismatchAtPosition,
    /// Lockstep comparison stops due to structural divergence.
    StructuralBreakAtPosition,
    /// Assistant message-level content differs.
    AssistantContentDifference,
    /// Provider final response differs.
    ProviderCallResponseDifference,
    /// Tool input differs.
    ToolCallInputDifference,
    /// Tool output differs.
    ToolCallOutputDifference,
    /// Permission decision differs.
    PermissionGateDecisionDifference,
    /// Session start cwd differs.
    SessionStartCwdDifference,
    /// Session end payload differs.
    SessionEndDifference,
    /// Generic hash/reference mismatch for a specific field.
    ContentReferenceDifference,
}

/// One diff divergence record with expected/actual summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffDivergence {
    /// Lockstep position where divergence occurred (when known).
    pub position: Option<u64>,
    /// Divergence category.
    pub kind: DiffDivergenceKind,
    /// Optional field name that differed.
    pub field: Option<String>,
    /// Expected value summary.
    pub expected: String,
    /// Actual value summary.
    pub actual: String,
}

#[cfg(test)]
mod tests {
    use super::DiffDivergenceKind;

    #[test]
    fn t_kind_serializes_to_expected_strings() {
        let cases = [
            (
                DiffDivergenceKind::EventCountMismatch,
                "event_count_mismatch",
            ),
            (
                DiffDivergenceKind::EventKindMismatchAtPosition,
                "event_kind_mismatch_at_position",
            ),
            (
                DiffDivergenceKind::StructuralBreakAtPosition,
                "structural_break_at_position",
            ),
            (
                DiffDivergenceKind::AssistantContentDifference,
                "assistant_content_difference",
            ),
            (
                DiffDivergenceKind::ProviderCallResponseDifference,
                "provider_call_response_difference",
            ),
            (
                DiffDivergenceKind::ToolCallInputDifference,
                "tool_call_input_difference",
            ),
            (
                DiffDivergenceKind::ToolCallOutputDifference,
                "tool_call_output_difference",
            ),
            (
                DiffDivergenceKind::PermissionGateDecisionDifference,
                "permission_gate_decision_difference",
            ),
            (
                DiffDivergenceKind::SessionStartCwdDifference,
                "session_start_cwd_difference",
            ),
            (
                DiffDivergenceKind::SessionEndDifference,
                "session_end_difference",
            ),
            (
                DiffDivergenceKind::ContentReferenceDifference,
                "content_reference_difference",
            ),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).expect("serialize");
            assert_eq!(json, format!("\"{expected}\""));
        }
    }
}
