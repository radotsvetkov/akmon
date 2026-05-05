use akmon_journal::{Event, EventKind, Hash};
use serde::{Deserialize, Serialize};

use crate::{DiffDivergence, DiffDivergenceKind, DiffMode};

/// Structural break details for lockstep diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralBreak {
    /// Lockstep position where structure diverged.
    pub position: u64,
    /// Expected structural value (kind/count detail).
    pub expected: String,
    /// Actual structural value (kind/count detail).
    pub actual: String,
}

/// Intermediate comparison artifact assembled before report rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffComparison {
    /// Session A UUID.
    pub session_a_id: String,
    /// Session B UUID.
    pub session_b_id: String,
    /// Diff mode in effect.
    pub mode: DiffMode,
    /// All accumulated divergences.
    pub divergences: Vec<DiffDivergence>,
    /// Structural break, if lockstep comparison stopped early.
    pub structural_break: Option<StructuralBreak>,
    /// Number of events compared before stopping.
    pub events_compared: usize,
}

impl DiffComparison {
    /// Creates an empty comparison payload.
    #[must_use]
    pub fn new(session_a_id: String, session_b_id: String, mode: DiffMode) -> Self {
        Self {
            session_a_id,
            session_b_id,
            mode,
            divergences: Vec::new(),
            structural_break: None,
            events_compared: 0,
        }
    }
}

fn hash_hex(hash: &Hash) -> String {
    hash.to_hex()
}

fn content_ref_diff(
    position: u64,
    field: &str,
    expected: String,
    actual: String,
) -> DiffDivergence {
    DiffDivergence {
        position: Some(position),
        kind: DiffDivergenceKind::ContentReferenceDifference,
        field: Some(field.to_owned()),
        expected,
        actual,
    }
}

/// Compares `SessionStart` events at one lockstep position.
#[must_use]
pub fn compare_session_start(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    let mut out = Vec::new();
    if let (
        EventKind::SessionStart {
            cwd_hash: a_cwd,
            config_hash: a_cfg,
        },
        EventKind::SessionStart {
            cwd_hash: b_cwd,
            config_hash: b_cfg,
        },
    ) = (&a.kind, &b.kind)
    {
        if a_cwd != b_cwd {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::SessionStartCwdDifference,
                field: Some("cwd_hash".to_owned()),
                expected: hash_hex(a_cwd),
                actual: hash_hex(b_cwd),
            });
        }
        if a_cfg != b_cfg {
            out.push(content_ref_diff(
                a.sequence,
                "config_hash",
                hash_hex(a_cfg),
                hash_hex(b_cfg),
            ));
        }
    }
    out
}

/// Compares `UserTurn` events at one lockstep position.
#[must_use]
pub fn compare_user_turn(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    match (&a.kind, &b.kind) {
        (EventKind::UserTurn { prompt_hash: p1 }, EventKind::UserTurn { prompt_hash: p2 })
            if p1 != p2 =>
        {
            vec![content_ref_diff(
                a.sequence,
                "prompt_hash",
                hash_hex(p1),
                hash_hex(p2),
            )]
        }
        _ => Vec::new(),
    }
}

/// Compares `ProviderCall` events at one lockstep position.
#[must_use]
pub fn compare_provider_call(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    let mut out = Vec::new();
    if let (
        EventKind::ProviderCall {
            provider_id: a_id,
            attempts: a_attempts,
            stream_hash: a_stream,
        },
        EventKind::ProviderCall {
            provider_id: b_id,
            attempts: b_attempts,
            stream_hash: b_stream,
        },
    ) = (&a.kind, &b.kind)
    {
        if a_id != b_id {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::ContentReferenceDifference,
                field: Some("provider_id".to_owned()),
                expected: a_id.clone(),
                actual: b_id.clone(),
            });
        }
        let a_final = a_attempts
            .last()
            .and_then(|attempt| attempt.response_hash.as_ref());
        let b_final = b_attempts
            .last()
            .and_then(|attempt| attempt.response_hash.as_ref());
        if a_final != b_final {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::ProviderCallResponseDifference,
                field: Some("attempts[-1].response_hash".to_owned()),
                expected: a_final.map_or_else(|| "none".to_owned(), hash_hex),
                actual: b_final.map_or_else(|| "none".to_owned(), hash_hex),
            });
        }
        if a_stream != b_stream {
            out.push(content_ref_diff(
                a.sequence,
                "stream_hash",
                a_stream
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), hash_hex),
                b_stream
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), hash_hex),
            ));
        }
    }
    out
}

/// Compares `AssistantTurn` events at one lockstep position.
#[must_use]
pub fn compare_assistant_turn(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    let mut out = Vec::new();
    if let (
        EventKind::AssistantTurn {
            message_hash: a_message,
            tool_calls_hash: a_tools,
        },
        EventKind::AssistantTurn {
            message_hash: b_message,
            tool_calls_hash: b_tools,
        },
    ) = (&a.kind, &b.kind)
    {
        if a_message != b_message {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::AssistantContentDifference,
                field: Some("message_hash".to_owned()),
                expected: hash_hex(a_message),
                actual: hash_hex(b_message),
            });
        }
        if a_tools != b_tools {
            out.push(content_ref_diff(
                a.sequence,
                "tool_calls_hash",
                a_tools.as_ref().map_or_else(|| "none".to_owned(), hash_hex),
                b_tools.as_ref().map_or_else(|| "none".to_owned(), hash_hex),
            ));
        }
    }
    out
}

/// Compares `ToolCall` events at one lockstep position.
#[must_use]
pub fn compare_tool_call(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    let mut out = Vec::new();
    if let (
        EventKind::ToolCall {
            tool_id: a_id,
            input_hash: a_input,
            output_hash: a_output,
            side_effects_hash: a_side_effects,
        },
        EventKind::ToolCall {
            tool_id: b_id,
            input_hash: b_input,
            output_hash: b_output,
            side_effects_hash: b_side_effects,
        },
    ) = (&a.kind, &b.kind)
    {
        if a_id != b_id {
            out.push(content_ref_diff(
                a.sequence,
                "tool_id",
                a_id.clone(),
                b_id.clone(),
            ));
        }
        if a_input != b_input {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::ToolCallInputDifference,
                field: Some("input_hash".to_owned()),
                expected: hash_hex(a_input),
                actual: hash_hex(b_input),
            });
        }
        if a_output != b_output {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::ToolCallOutputDifference,
                field: Some("output_hash".to_owned()),
                expected: hash_hex(a_output),
                actual: hash_hex(b_output),
            });
        }
        if a_side_effects != b_side_effects {
            out.push(content_ref_diff(
                a.sequence,
                "side_effects_hash",
                a_side_effects
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), hash_hex),
                b_side_effects
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), hash_hex),
            ));
        }
    }
    out
}

/// Compares `PermissionGate` events at one lockstep position.
#[must_use]
pub fn compare_permission_gate(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    let mut out = Vec::new();
    if let (
        EventKind::PermissionGate {
            policy_id: a_policy,
            decision: a_decision,
            context_hash: a_context,
        },
        EventKind::PermissionGate {
            policy_id: b_policy,
            decision: b_decision,
            context_hash: b_context,
        },
    ) = (&a.kind, &b.kind)
    {
        if a_policy != b_policy {
            out.push(content_ref_diff(
                a.sequence,
                "policy_id",
                a_policy.clone(),
                b_policy.clone(),
            ));
        }
        if a_decision != b_decision {
            out.push(DiffDivergence {
                position: Some(a.sequence),
                kind: DiffDivergenceKind::PermissionGateDecisionDifference,
                field: Some("decision".to_owned()),
                expected: a_decision.clone(),
                actual: b_decision.clone(),
            });
        }
        if a_context != b_context {
            out.push(content_ref_diff(
                a.sequence,
                "context_hash",
                hash_hex(a_context),
                hash_hex(b_context),
            ));
        }
    }
    out
}

/// Compares `SessionEnd` events at one lockstep position.
#[must_use]
pub fn compare_session_end(a: &Event, b: &Event) -> Vec<DiffDivergence> {
    if let (
        EventKind::SessionEnd {
            summary_hash: a_summary,
        },
        EventKind::SessionEnd {
            summary_hash: b_summary,
        },
    ) = (&a.kind, &b.kind)
        && a_summary != b_summary
    {
        return vec![DiffDivergence {
            position: Some(a.sequence),
            kind: DiffDivergenceKind::SessionEndDifference,
            field: Some("summary_hash".to_owned()),
            expected: a_summary
                .as_ref()
                .map_or_else(|| "none".to_owned(), hash_hex),
            actual: b_summary
                .as_ref()
                .map_or_else(|| "none".to_owned(), hash_hex),
        }];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use akmon_journal::{AttemptRecord, AttemptStatus, HashAlgorithm};
    use time::OffsetDateTime;

    use super::*;

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes(HashAlgorithm::Sha256, [byte; 32])
    }

    fn event(sequence: u64, kind: EventKind) -> Event {
        Event {
            parents: Vec::new(),
            kind,
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence,
        }
    }

    fn attempt(n: u32, response_hash: Option<Hash>) -> AttemptRecord {
        AttemptRecord {
            attempt_number: n,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash: hash(10 + n as u8),
            response_hash,
            stream_hash: None,
            error_message: None,
        }
    }

    #[test]
    fn t_compare_session_start_identical_no_divergences() {
        let a = event(
            0,
            EventKind::SessionStart {
                cwd_hash: hash(1),
                config_hash: hash(2),
            },
        );
        let b = a.clone();
        assert!(compare_session_start(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_session_start_detects_cwd_and_config() {
        let a = event(
            0,
            EventKind::SessionStart {
                cwd_hash: hash(1),
                config_hash: hash(2),
            },
        );
        let b = event(
            0,
            EventKind::SessionStart {
                cwd_hash: hash(3),
                config_hash: hash(4),
            },
        );
        let diffs = compare_session_start(&a, &b);
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].kind, DiffDivergenceKind::SessionStartCwdDifference);
        assert_eq!(
            diffs[1].kind,
            DiffDivergenceKind::ContentReferenceDifference
        );
    }

    #[test]
    fn t_compare_user_turn_identical_no_divergences() {
        let a = event(
            1,
            EventKind::UserTurn {
                prompt_hash: hash(5),
            },
        );
        let b = a.clone();
        assert!(compare_user_turn(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_user_turn_detects_prompt_hash_difference() {
        let a = event(
            1,
            EventKind::UserTurn {
                prompt_hash: hash(5),
            },
        );
        let b = event(
            1,
            EventKind::UserTurn {
                prompt_hash: hash(6),
            },
        );
        let diffs = compare_user_turn(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].field.as_deref(), Some("prompt_hash"));
    }

    #[test]
    fn t_compare_provider_call_identical_no_divergences() {
        let a = event(
            2,
            EventKind::ProviderCall {
                provider_id: "anthropic".to_owned(),
                attempts: vec![attempt(1, Some(hash(7)))],
                stream_hash: Some(hash(8)),
            },
        );
        let b = a.clone();
        assert!(compare_provider_call(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_provider_call_detects_provider_response_and_stream() {
        let a = event(
            2,
            EventKind::ProviderCall {
                provider_id: "anthropic".to_owned(),
                attempts: vec![attempt(1, Some(hash(7)))],
                stream_hash: Some(hash(8)),
            },
        );
        let b = event(
            2,
            EventKind::ProviderCall {
                provider_id: "openai".to_owned(),
                attempts: vec![attempt(1, Some(hash(9)))],
                stream_hash: None,
            },
        );
        let diffs = compare_provider_call(&a, &b);
        assert_eq!(diffs.len(), 3);
        assert_eq!(
            diffs[1].kind,
            DiffDivergenceKind::ProviderCallResponseDifference
        );
    }

    #[test]
    fn t_compare_assistant_turn_identical_no_divergences() {
        let a = event(
            3,
            EventKind::AssistantTurn {
                message_hash: hash(10),
                tool_calls_hash: Some(hash(11)),
            },
        );
        let b = a.clone();
        assert!(compare_assistant_turn(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_assistant_turn_detects_message_and_tool_calls() {
        let a = event(
            3,
            EventKind::AssistantTurn {
                message_hash: hash(10),
                tool_calls_hash: Some(hash(11)),
            },
        );
        let b = event(
            3,
            EventKind::AssistantTurn {
                message_hash: hash(12),
                tool_calls_hash: None,
            },
        );
        let diffs = compare_assistant_turn(&a, &b);
        assert_eq!(diffs.len(), 2);
        assert_eq!(
            diffs[0].kind,
            DiffDivergenceKind::AssistantContentDifference
        );
    }

    #[test]
    fn t_compare_tool_call_identical_no_divergences() {
        let a = event(
            4,
            EventKind::ToolCall {
                tool_id: "read_file".to_owned(),
                input_hash: hash(13),
                output_hash: hash(14),
                side_effects_hash: Some(hash(15)),
            },
        );
        let b = a.clone();
        assert!(compare_tool_call(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_tool_call_detects_main_differences() {
        let a = event(
            4,
            EventKind::ToolCall {
                tool_id: "read_file".to_owned(),
                input_hash: hash(13),
                output_hash: hash(14),
                side_effects_hash: Some(hash(15)),
            },
        );
        let b = event(
            4,
            EventKind::ToolCall {
                tool_id: "write_file".to_owned(),
                input_hash: hash(16),
                output_hash: hash(17),
                side_effects_hash: None,
            },
        );
        let diffs = compare_tool_call(&a, &b);
        assert_eq!(diffs.len(), 4);
        assert_eq!(diffs[1].kind, DiffDivergenceKind::ToolCallInputDifference);
        assert_eq!(diffs[2].kind, DiffDivergenceKind::ToolCallOutputDifference);
    }

    #[test]
    fn t_compare_permission_gate_identical_no_divergences() {
        let a = event(
            5,
            EventKind::PermissionGate {
                policy_id: "default".to_owned(),
                decision: "allow".to_owned(),
                context_hash: hash(18),
            },
        );
        let b = a.clone();
        assert!(compare_permission_gate(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_permission_gate_detects_policy_decision_context() {
        let a = event(
            5,
            EventKind::PermissionGate {
                policy_id: "default".to_owned(),
                decision: "allow".to_owned(),
                context_hash: hash(18),
            },
        );
        let b = event(
            5,
            EventKind::PermissionGate {
                policy_id: "strict".to_owned(),
                decision: "deny".to_owned(),
                context_hash: hash(19),
            },
        );
        let diffs = compare_permission_gate(&a, &b);
        assert_eq!(diffs.len(), 3);
        assert_eq!(
            diffs[1].kind,
            DiffDivergenceKind::PermissionGateDecisionDifference
        );
    }

    #[test]
    fn t_compare_session_end_identical_no_divergences() {
        let a = event(
            6,
            EventKind::SessionEnd {
                summary_hash: Some(hash(20)),
            },
        );
        let b = a.clone();
        assert!(compare_session_end(&a, &b).is_empty());
    }

    #[test]
    fn t_compare_session_end_detects_difference() {
        let a = event(
            6,
            EventKind::SessionEnd {
                summary_hash: Some(hash(20)),
            },
        );
        let b = event(6, EventKind::SessionEnd { summary_hash: None });
        let diffs = compare_session_end(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].kind, DiffDivergenceKind::SessionEndDifference);
    }

    #[test]
    fn t_diff_comparison_new_initializes_empty() {
        let comparison = DiffComparison::new("a".to_owned(), "b".to_owned(), DiffMode::Default);
        assert_eq!(comparison.events_compared, 0);
        assert!(comparison.divergences.is_empty());
        assert!(comparison.structural_break.is_none());
    }
}
