use akmon_journal::{Event, EventKind, Hash};
use serde::{Deserialize, Serialize};

use crate::resolve::ResolveContext;
use crate::{DiffDivergence, DiffDivergenceKind, DiffError, DiffMode};

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

fn divergence(
    position: Option<u64>,
    kind: DiffDivergenceKind,
    field: Option<String>,
    expected: String,
    actual: String,
) -> DiffDivergence {
    DiffDivergence {
        position,
        kind,
        field,
        expected,
        actual,
        resolved: None,
        resolved_skip_reason: None,
    }
}

fn content_ref_diff(
    position: u64,
    field: &str,
    expected: String,
    actual: String,
) -> DiffDivergence {
    divergence(
        Some(position),
        DiffDivergenceKind::ContentReferenceDifference,
        Some(field.to_owned()),
        expected,
        actual,
    )
}

/// Compares `SessionStart` events at one lockstep position.
pub fn compare_session_start(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::attach_resolved_content_pair;

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
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::SessionStartCwdDifference,
                Some("cwd_hash".to_owned()),
                hash_hex(a_cwd),
                hash_hex(b_cwd),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(out.last_mut().expect("pushed"), ctx, a_cwd, b_cwd)?;
            }
        }
        if a_cfg != b_cfg {
            out.push(content_ref_diff(
                a.sequence,
                "config_hash",
                hash_hex(a_cfg),
                hash_hex(b_cfg),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(out.last_mut().expect("pushed"), ctx, a_cfg, b_cfg)?;
            }
        }
    }
    Ok(out)
}

/// Compares `UserTurn` events at one lockstep position.
pub fn compare_user_turn(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::attach_resolved_content_pair;

    match (&a.kind, &b.kind) {
        (EventKind::UserTurn { prompt_hash: p1 }, EventKind::UserTurn { prompt_hash: p2 })
            if p1 != p2 =>
        {
            let mut out = vec![content_ref_diff(
                a.sequence,
                "prompt_hash",
                hash_hex(p1),
                hash_hex(p2),
            )];
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(&mut out[0], ctx, p1, p2)?;
            }
            Ok(out)
        }
        _ => Ok(Vec::new()),
    }
}

/// Compares `ProviderCall` events at one lockstep position.
pub fn compare_provider_call(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::{attach_resolved_content_pair, mark_not_dereferenceable};

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
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::ContentReferenceDifference,
                Some("provider_id".to_owned()),
                a_id.clone(),
                b_id.clone(),
            ));
            if resolve.is_some() {
                mark_not_dereferenceable(out.last_mut().expect("pushed"));
            }
        }
        let a_final = a_attempts
            .last()
            .and_then(|attempt| attempt.response_hash.as_ref());
        let b_final = b_attempts
            .last()
            .and_then(|attempt| attempt.response_hash.as_ref());
        if a_final != b_final {
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::ProviderCallResponseDifference,
                Some("attempts[-1].response_hash".to_owned()),
                a_final.map_or_else(|| "none".to_owned(), hash_hex),
                b_final.map_or_else(|| "none".to_owned(), hash_hex),
            ));
            if let Some(ctx) = resolve {
                match (a_final, b_final) {
                    (Some(ha), Some(hb)) => {
                        attach_resolved_content_pair(out.last_mut().expect("pushed"), ctx, ha, hb)?;
                    }
                    _ => mark_not_dereferenceable(out.last_mut().expect("pushed")),
                }
            }
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
            if let Some(ctx) = resolve {
                match (a_stream.as_ref(), b_stream.as_ref()) {
                    (Some(ha), Some(hb)) => {
                        attach_resolved_content_pair(out.last_mut().expect("pushed"), ctx, ha, hb)?;
                    }
                    _ => mark_not_dereferenceable(out.last_mut().expect("pushed")),
                }
            }
        }
    }
    Ok(out)
}

/// Compares `AssistantTurn` events at one lockstep position.
pub fn compare_assistant_turn(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::{attach_resolved_content_pair, mark_not_dereferenceable};

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
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::AssistantContentDifference,
                Some("message_hash".to_owned()),
                hash_hex(a_message),
                hash_hex(b_message),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(
                    out.last_mut().expect("pushed"),
                    ctx,
                    a_message,
                    b_message,
                )?;
            }
        }
        if a_tools != b_tools {
            out.push(content_ref_diff(
                a.sequence,
                "tool_calls_hash",
                a_tools.as_ref().map_or_else(|| "none".to_owned(), hash_hex),
                b_tools.as_ref().map_or_else(|| "none".to_owned(), hash_hex),
            ));
            if let Some(ctx) = resolve {
                match (a_tools.as_ref(), b_tools.as_ref()) {
                    (Some(ha), Some(hb)) => {
                        attach_resolved_content_pair(out.last_mut().expect("pushed"), ctx, ha, hb)?;
                    }
                    _ => mark_not_dereferenceable(out.last_mut().expect("pushed")),
                }
            }
        }
    }
    Ok(out)
}

/// Compares `ToolCall` events at one lockstep position.
pub fn compare_tool_call(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::{attach_resolved_content_pair, mark_not_dereferenceable};

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
            if resolve.is_some() {
                mark_not_dereferenceable(out.last_mut().expect("pushed"));
            }
        }
        if a_input != b_input {
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::ToolCallInputDifference,
                Some("input_hash".to_owned()),
                hash_hex(a_input),
                hash_hex(b_input),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(
                    out.last_mut().expect("pushed"),
                    ctx,
                    a_input,
                    b_input,
                )?;
            }
        }
        if a_output != b_output {
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::ToolCallOutputDifference,
                Some("output_hash".to_owned()),
                hash_hex(a_output),
                hash_hex(b_output),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(
                    out.last_mut().expect("pushed"),
                    ctx,
                    a_output,
                    b_output,
                )?;
            }
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
            if let Some(ctx) = resolve {
                match (a_side_effects.as_ref(), b_side_effects.as_ref()) {
                    (Some(ha), Some(hb)) => {
                        attach_resolved_content_pair(out.last_mut().expect("pushed"), ctx, ha, hb)?;
                    }
                    _ => mark_not_dereferenceable(out.last_mut().expect("pushed")),
                }
            }
        }
    }
    Ok(out)
}

/// Compares `RetrievalCall` events at one lockstep position.
pub fn compare_retrieval_call(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::{attach_resolved_content_pair, mark_not_dereferenceable};

    let mut out = Vec::new();
    if let (
        EventKind::RetrievalCall {
            index_id: a_index,
            query_hash: a_query,
            results_hash: a_results,
        },
        EventKind::RetrievalCall {
            index_id: b_index,
            query_hash: b_query,
            results_hash: b_results,
        },
    ) = (&a.kind, &b.kind)
    {
        if a_index != b_index {
            out.push(content_ref_diff(
                a.sequence,
                "index_id",
                a_index.clone(),
                b_index.clone(),
            ));
            if resolve.is_some() {
                mark_not_dereferenceable(out.last_mut().expect("pushed"));
            }
        }
        if a_query != b_query {
            out.push(content_ref_diff(
                a.sequence,
                "query_hash",
                hash_hex(a_query),
                hash_hex(b_query),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(
                    out.last_mut().expect("pushed"),
                    ctx,
                    a_query,
                    b_query,
                )?;
            }
        }
        if a_results != b_results {
            out.push(content_ref_diff(
                a.sequence,
                "results_hash",
                hash_hex(a_results),
                hash_hex(b_results),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(
                    out.last_mut().expect("pushed"),
                    ctx,
                    a_results,
                    b_results,
                )?;
            }
        }
    }
    Ok(out)
}

/// Compares `PermissionGate` events at one lockstep position.
pub fn compare_permission_gate(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::{attach_resolved_content_pair, mark_not_dereferenceable};

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
            if resolve.is_some() {
                mark_not_dereferenceable(out.last_mut().expect("pushed"));
            }
        }
        if a_decision != b_decision {
            out.push(divergence(
                Some(a.sequence),
                DiffDivergenceKind::PermissionGateDecisionDifference,
                Some("decision".to_owned()),
                a_decision.clone(),
                b_decision.clone(),
            ));
            if resolve.is_some() {
                mark_not_dereferenceable(out.last_mut().expect("pushed"));
            }
        }
        if a_context != b_context {
            out.push(content_ref_diff(
                a.sequence,
                "context_hash",
                hash_hex(a_context),
                hash_hex(b_context),
            ));
            if let Some(ctx) = resolve {
                attach_resolved_content_pair(
                    out.last_mut().expect("pushed"),
                    ctx,
                    a_context,
                    b_context,
                )?;
            }
        }
    }
    Ok(out)
}

/// Compares `SessionEnd` events at one lockstep position.
pub fn compare_session_end(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
    use crate::resolve::{attach_resolved_content_pair, mark_not_dereferenceable};

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
        let mut out = vec![divergence(
            Some(a.sequence),
            DiffDivergenceKind::SessionEndDifference,
            Some("summary_hash".to_owned()),
            a_summary
                .as_ref()
                .map_or_else(|| "none".to_owned(), hash_hex),
            b_summary
                .as_ref()
                .map_or_else(|| "none".to_owned(), hash_hex),
        )];
        if let Some(ctx) = resolve {
            match (a_summary.as_ref(), b_summary.as_ref()) {
                (Some(ha), Some(hb)) => {
                    attach_resolved_content_pair(&mut out[0], ctx, ha, hb)?;
                }
                _ => mark_not_dereferenceable(&mut out[0]),
            }
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use akmon_journal::{
        AttemptRecord, AttemptStatus, HashAlgorithm, MemoryObjectStore, ObjectStore,
    };
    use time::OffsetDateTime;

    use crate::resolve::{
        RESOLVE_READ_CAP_BYTES, RESOLVE_SKIP_EXCEEDS_CAP, RESOLVE_SKIP_NOT_DEREFERENCABLE,
        RESOLVE_SKIP_OBJECT_MISSING, ResolveContext,
    };

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
        assert!(
            compare_session_start(&a, &b, None)
                .expect("compare")
                .is_empty()
        );
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
        let diffs = compare_session_start(&a, &b, None).expect("compare");
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
        assert!(compare_user_turn(&a, &b, None).expect("compare").is_empty());
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
        let diffs = compare_user_turn(&a, &b, None).expect("compare");
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
        assert!(
            compare_provider_call(&a, &b, None)
                .expect("compare")
                .is_empty()
        );
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
        let diffs = compare_provider_call(&a, &b, None).expect("compare");
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
        assert!(
            compare_assistant_turn(&a, &b, None)
                .expect("compare")
                .is_empty()
        );
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
        let diffs = compare_assistant_turn(&a, &b, None).expect("compare");
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
        assert!(compare_tool_call(&a, &b, None).expect("compare").is_empty());
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
        let diffs = compare_tool_call(&a, &b, None).expect("compare");
        assert_eq!(diffs.len(), 4);
        assert_eq!(diffs[1].kind, DiffDivergenceKind::ToolCallInputDifference);
        assert_eq!(diffs[2].kind, DiffDivergenceKind::ToolCallOutputDifference);
    }

    #[test]
    fn t_compare_retrieval_call_identical_no_divergences() {
        let a = event(
            4,
            EventKind::RetrievalCall {
                index_id: "idx".to_owned(),
                query_hash: hash(30),
                results_hash: hash(31),
            },
        );
        let b = a.clone();
        assert!(
            compare_retrieval_call(&a, &b, None)
                .expect("compare")
                .is_empty()
        );
    }

    #[test]
    fn t_compare_retrieval_call_detects_field_differences() {
        let a = event(
            4,
            EventKind::RetrievalCall {
                index_id: "idx-a".to_owned(),
                query_hash: hash(30),
                results_hash: hash(31),
            },
        );
        let b = event(
            4,
            EventKind::RetrievalCall {
                index_id: "idx-b".to_owned(),
                query_hash: hash(32),
                results_hash: hash(33),
            },
        );
        let diffs = compare_retrieval_call(&a, &b, None).expect("compare");
        assert_eq!(diffs.len(), 3);
        assert_eq!(diffs[0].field.as_deref(), Some("index_id"));
        assert_eq!(diffs[1].field.as_deref(), Some("query_hash"));
        assert_eq!(diffs[2].field.as_deref(), Some("results_hash"));
        assert_eq!(
            diffs[0].kind,
            DiffDivergenceKind::ContentReferenceDifference
        );
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
        assert!(
            compare_permission_gate(&a, &b, None)
                .expect("compare")
                .is_empty()
        );
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
        let diffs = compare_permission_gate(&a, &b, None).expect("compare");
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
        assert!(
            compare_session_end(&a, &b, None)
                .expect("compare")
                .is_empty()
        );
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
        let diffs = compare_session_end(&a, &b, None).expect("compare");
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

    #[test]
    fn t_resolve_user_turn_loads_prompt_bytes() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let pa = sa.put(b"hello-a").expect("put");
        let pb = sb.put(b"hello-b").expect("put");
        let a = event(1, EventKind::UserTurn { prompt_hash: pa });
        let b = event(1, EventKind::UserTurn { prompt_hash: pb });
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_user_turn(&a, &b, Some(ctx)).expect("compare");
        let r = diffs[0].resolved.as_ref().expect("resolved");
        assert!(!r.bytes_match);
        assert_eq!(r.a_size_bytes, 7);
        assert_eq!(r.b_size_bytes, 7);
        assert!(diffs[0].resolved_skip_reason.is_none());
    }

    #[test]
    fn t_resolve_user_turn_skips_when_object_missing() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let pa = sa.put(b"a").expect("put");
        let pb = sb.put(b"b").expect("put");
        sb.remove_object_for_testing(&pb).expect("remove");
        let a = event(1, EventKind::UserTurn { prompt_hash: pa });
        let b = event(1, EventKind::UserTurn { prompt_hash: pb });
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_user_turn(&a, &b, Some(ctx)).expect("compare");
        assert!(diffs[0].resolved.is_none());
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_OBJECT_MISSING)
        );
    }

    #[test]
    fn t_resolve_user_turn_skips_when_over_cap() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let big = vec![0u8; RESOLVE_READ_CAP_BYTES + 1];
        let pa = sa.put(&big).expect("put");
        let pb = sb.put(b"small").expect("put");
        let a = event(1, EventKind::UserTurn { prompt_hash: pa });
        let b = event(1, EventKind::UserTurn { prompt_hash: pb });
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_user_turn(&a, &b, Some(ctx)).expect("compare");
        assert!(diffs[0].resolved.is_none());
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_EXCEEDS_CAP)
        );
    }

    #[test]
    fn t_resolve_session_start_cwd_attaches_bytes() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let ca = sa.put(b"/a").expect("cwd");
        let cb = sb.put(b"/b").expect("cwd");
        let cfg = sa.put(b"cfg").expect("cfg");
        let cfg_b = sb.put(b"cfg").expect("cfg");
        let a = event(
            0,
            EventKind::SessionStart {
                cwd_hash: ca,
                config_hash: cfg,
            },
        );
        let b = event(
            0,
            EventKind::SessionStart {
                cwd_hash: cb,
                config_hash: cfg_b,
            },
        );
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_session_start(&a, &b, Some(ctx)).expect("compare");
        assert!(diffs[0].resolved.as_ref().is_some_and(|r| !r.bytes_match));
    }

    #[test]
    fn t_resolve_provider_id_not_dereferenceable() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let a = event(
            2,
            EventKind::ProviderCall {
                provider_id: "p1".to_owned(),
                attempts: vec![attempt(1, Some(hash(1)))],
                stream_hash: None,
            },
        );
        let b = event(
            2,
            EventKind::ProviderCall {
                provider_id: "p2".to_owned(),
                attempts: vec![attempt(1, Some(hash(1)))],
                stream_hash: None,
            },
        );
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_provider_call(&a, &b, Some(ctx)).expect("compare");
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
        );
    }

    #[test]
    fn t_resolve_tool_id_not_dereferenceable() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let a = event(
            4,
            EventKind::ToolCall {
                tool_id: "a".to_owned(),
                input_hash: hash(1),
                output_hash: hash(2),
                side_effects_hash: None,
            },
        );
        let b = event(
            4,
            EventKind::ToolCall {
                tool_id: "b".to_owned(),
                input_hash: hash(1),
                output_hash: hash(2),
                side_effects_hash: None,
            },
        );
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_tool_call(&a, &b, Some(ctx)).expect("compare");
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
        );
    }

    #[test]
    fn t_resolve_retrieval_index_not_dereferenceable() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let a = event(
            3,
            EventKind::RetrievalCall {
                index_id: "i1".to_owned(),
                query_hash: hash(1),
                results_hash: hash(2),
            },
        );
        let b = event(
            3,
            EventKind::RetrievalCall {
                index_id: "i2".to_owned(),
                query_hash: hash(1),
                results_hash: hash(2),
            },
        );
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_retrieval_call(&a, &b, Some(ctx)).expect("compare");
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
        );
    }

    #[test]
    fn t_resolve_permission_policy_not_dereferenceable() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let a = event(
            5,
            EventKind::PermissionGate {
                policy_id: "p1".to_owned(),
                decision: "allow".to_owned(),
                context_hash: hash(1),
            },
        );
        let b = event(
            5,
            EventKind::PermissionGate {
                policy_id: "p2".to_owned(),
                decision: "allow".to_owned(),
                context_hash: hash(1),
            },
        );
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_permission_gate(&a, &b, Some(ctx)).expect("compare");
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
        );
    }

    #[test]
    fn t_resolve_session_end_skips_when_one_summary_none() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let a = event(
            6,
            EventKind::SessionEnd {
                summary_hash: Some(hash(1)),
            },
        );
        let b = event(6, EventKind::SessionEnd { summary_hash: None });
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_session_end(&a, &b, Some(ctx)).expect("compare");
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
        );
    }

    #[test]
    fn t_resolve_assistant_tool_calls_optional_pair_resolves() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let ha = sa.put(b"t1").expect("put");
        let hb = sb.put(b"t2").expect("put");
        let a = event(
            3,
            EventKind::AssistantTurn {
                message_hash: hash(10),
                tool_calls_hash: Some(ha),
            },
        );
        let b = event(
            3,
            EventKind::AssistantTurn {
                message_hash: hash(10),
                tool_calls_hash: Some(hb),
            },
        );
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_assistant_turn(&a, &b, Some(ctx)).expect("compare");
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].resolved.as_ref().is_some_and(|r| !r.bytes_match));
    }
}
