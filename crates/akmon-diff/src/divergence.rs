use serde::{Deserialize, Serialize};

/// Byte-level summary for a resolved hash-field divergence (`--resolve` mode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContent {
    /// Size of session A object bytes.
    pub a_size_bytes: usize,
    /// Size of session B object bytes.
    pub b_size_bytes: usize,
    /// Preview for A (UTF-8 or hex).
    pub a_preview: Option<String>,
    /// Preview for B (UTF-8 or hex).
    pub b_preview: Option<String>,
    /// True when loaded bytes are identical (investigate when hashes differ).
    pub bytes_match: bool,
}

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
    /// Resolved byte summary when `run_with_resolve` populated this row.
    pub resolved: Option<ResolvedContent>,
    /// Skip reason when resolve was requested but bytes were not attached.
    pub resolved_skip_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{DiffDivergenceKind, ResolvedContent};

    #[test]
    fn t_resolved_content_json_round_trip() {
        let value = ResolvedContent {
            a_size_bytes: 3,
            b_size_bytes: 4,
            a_preview: Some("abc".to_owned()),
            b_preview: None,
            bytes_match: false,
        };
        let json = serde_json::to_string(&value).expect("serialize");
        let back: ResolvedContent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, value);
        let v: serde_json::Value = serde_json::from_str(&json).expect("value");
        assert_eq!(v["a_size_bytes"], 3);
        assert_eq!(v["bytes_match"], false);
    }

    #[test]
    fn t_divergence_json_includes_resolved_nullable() {
        let d = super::DiffDivergence {
            position: Some(1),
            kind: DiffDivergenceKind::ContentReferenceDifference,
            field: Some("prompt_hash".to_owned()),
            expected: "a".to_owned(),
            actual: "b".to_owned(),
            resolved: None,
            resolved_skip_reason: None,
        };
        let json = serde_json::to_string(&d).expect("serialize");
        assert!(json.contains("\"resolved\":null"));
        assert!(json.contains("\"resolved_skip_reason\":null"));
    }

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
