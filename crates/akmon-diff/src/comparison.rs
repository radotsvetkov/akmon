use akmon_journal::{AttemptRecord, AttemptStatus, Event, EventKind, Hash};
use serde::{Deserialize, Serialize};

use crate::resolve::{ResolveContext, attach_resolved_content_pair, mark_not_dereferenceable};
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

fn opt_hash_hex(hash: Option<&Hash>) -> String {
    hash.map_or_else(|| "none".to_owned(), hash_hex)
}

fn divergence(
    position: u64,
    kind: DiffDivergenceKind,
    field: &str,
    expected: String,
    actual: String,
) -> DiffDivergence {
    DiffDivergence {
        position: Some(position),
        kind,
        field: Some(field.to_owned()),
        expected,
        actual,
        resolved: None,
        resolved_skip_reason: None,
    }
}

/// Pushes a divergence for a hash-backed field; in resolve mode loads both objects when present.
fn push_hash_field(
    out: &mut Vec<DiffDivergence>,
    mut div: DiffDivergence,
    resolve: Option<ResolveContext<'_>>,
    a: Option<&Hash>,
    b: Option<&Hash>,
) -> Result<(), DiffError> {
    if let Some(ctx) = resolve {
        match (a, b) {
            (Some(ha), Some(hb)) => attach_resolved_content_pair(&mut div, ctx, ha, hb)?,
            _ => mark_not_dereferenceable(&mut div),
        }
    }
    out.push(div);
    Ok(())
}

/// Pushes a divergence for a scalar (non-hash) field; resolve mode marks it not dereferenceable.
fn push_scalar_field(
    out: &mut Vec<DiffDivergence>,
    mut div: DiffDivergence,
    resolve: Option<ResolveContext<'_>>,
) {
    if resolve.is_some() {
        mark_not_dereferenceable(&mut div);
    }
    out.push(div);
}

/// Stable snake_case label for an attempt status (used in divergence expected/actual).
fn attempt_status_label(status: &AttemptStatus) -> String {
    match status {
        AttemptStatus::Success => "success".to_owned(),
        AttemptStatus::RateLimited => "rate_limited".to_owned(),
        AttemptStatus::NetworkError => "network_error".to_owned(),
        AttemptStatus::ServerError => "server_error".to_owned(),
        AttemptStatus::ClientError => "client_error".to_owned(),
        AttemptStatus::Cancelled => "cancelled".to_owned(),
        AttemptStatus::Other(other) => format!("other:{other}"),
    }
}

/// Compares `SessionStart` events at one lockstep position.
pub fn compare_session_start(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
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
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::SessionStartCwdDifference,
                "cwd_hash",
                hash_hex(a_cwd),
                hash_hex(b_cwd),
            );
            push_hash_field(&mut out, div, resolve, Some(a_cwd), Some(b_cwd))?;
        }
        if a_cfg != b_cfg {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "config_hash",
                hash_hex(a_cfg),
                hash_hex(b_cfg),
            );
            push_hash_field(&mut out, div, resolve, Some(a_cfg), Some(b_cfg))?;
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
    let mut out = Vec::new();
    if let (EventKind::UserTurn { prompt_hash: p1 }, EventKind::UserTurn { prompt_hash: p2 }) =
        (&a.kind, &b.kind)
        && p1 != p2
    {
        let div = divergence(
            a.sequence,
            DiffDivergenceKind::ContentReferenceDifference,
            "prompt_hash",
            hash_hex(p1),
            hash_hex(p2),
        );
        push_hash_field(&mut out, div, resolve, Some(p1), Some(p2))?;
    }
    Ok(out)
}

/// Compares the per-attempt request hash, status and error message of two attempts at `index`.
fn compare_attempt_pair(
    out: &mut Vec<DiffDivergence>,
    position: u64,
    index: usize,
    a: &AttemptRecord,
    b: &AttemptRecord,
    resolve: Option<ResolveContext<'_>>,
) -> Result<(), DiffError> {
    if a.request_hash != b.request_hash {
        let div = divergence(
            position,
            DiffDivergenceKind::ProviderCallRequestDifference,
            &format!("attempts[{index}].request_hash"),
            hash_hex(&a.request_hash),
            hash_hex(&b.request_hash),
        );
        push_hash_field(
            out,
            div,
            resolve,
            Some(&a.request_hash),
            Some(&b.request_hash),
        )?;
    }
    if a.status != b.status {
        let div = divergence(
            position,
            DiffDivergenceKind::ProviderCallAttemptStatusDifference,
            &format!("attempts[{index}].status"),
            attempt_status_label(&a.status),
            attempt_status_label(&b.status),
        );
        push_scalar_field(out, div, resolve);
    }
    if a.error_message != b.error_message {
        let div = divergence(
            position,
            DiffDivergenceKind::ProviderCallAttemptErrorDifference,
            &format!("attempts[{index}].error_message"),
            a.error_message.clone().unwrap_or_else(|| "none".to_owned()),
            b.error_message.clone().unwrap_or_else(|| "none".to_owned()),
        );
        push_scalar_field(out, div, resolve);
    }
    Ok(())
}

/// Compares `ProviderCall` events at one lockstep position.
///
/// Compared fields, in emitted order: `provider_id`, attempt count, then for every attempt index
/// present in both events its `request_hash`, `status` and `error_message`, then the final
/// attempt's `response_hash` and the event-level `stream_hash`.
pub fn compare_provider_call(
    a: &Event,
    b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<DiffDivergence>, DiffError> {
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
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "provider_id",
                a_id.clone(),
                b_id.clone(),
            );
            push_scalar_field(&mut out, div, resolve);
        }
        if a_attempts.len() != b_attempts.len() {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ProviderCallAttemptCountDifference,
                "attempts.len",
                a_attempts.len().to_string(),
                b_attempts.len().to_string(),
            );
            push_scalar_field(&mut out, div, resolve);
        }
        for (index, (attempt_a, attempt_b)) in a_attempts.iter().zip(b_attempts.iter()).enumerate()
        {
            compare_attempt_pair(&mut out, a.sequence, index, attempt_a, attempt_b, resolve)?;
        }
        let a_final = a_attempts
            .last()
            .and_then(|attempt| attempt.response_hash.as_ref());
        let b_final = b_attempts
            .last()
            .and_then(|attempt| attempt.response_hash.as_ref());
        if a_final != b_final {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ProviderCallResponseDifference,
                "attempts[-1].response_hash",
                opt_hash_hex(a_final),
                opt_hash_hex(b_final),
            );
            push_hash_field(&mut out, div, resolve, a_final, b_final)?;
        }
        if a_stream != b_stream {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "stream_hash",
                opt_hash_hex(a_stream.as_ref()),
                opt_hash_hex(b_stream.as_ref()),
            );
            push_hash_field(&mut out, div, resolve, a_stream.as_ref(), b_stream.as_ref())?;
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
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::AssistantContentDifference,
                "message_hash",
                hash_hex(a_message),
                hash_hex(b_message),
            );
            push_hash_field(&mut out, div, resolve, Some(a_message), Some(b_message))?;
        }
        if a_tools != b_tools {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "tool_calls_hash",
                opt_hash_hex(a_tools.as_ref()),
                opt_hash_hex(b_tools.as_ref()),
            );
            push_hash_field(&mut out, div, resolve, a_tools.as_ref(), b_tools.as_ref())?;
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
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "tool_id",
                a_id.clone(),
                b_id.clone(),
            );
            push_scalar_field(&mut out, div, resolve);
        }
        if a_input != b_input {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ToolCallInputDifference,
                "input_hash",
                hash_hex(a_input),
                hash_hex(b_input),
            );
            push_hash_field(&mut out, div, resolve, Some(a_input), Some(b_input))?;
        }
        if a_output != b_output {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ToolCallOutputDifference,
                "output_hash",
                hash_hex(a_output),
                hash_hex(b_output),
            );
            push_hash_field(&mut out, div, resolve, Some(a_output), Some(b_output))?;
        }
        if a_side_effects != b_side_effects {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "side_effects_hash",
                opt_hash_hex(a_side_effects.as_ref()),
                opt_hash_hex(b_side_effects.as_ref()),
            );
            push_hash_field(
                &mut out,
                div,
                resolve,
                a_side_effects.as_ref(),
                b_side_effects.as_ref(),
            )?;
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
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "index_id",
                a_index.clone(),
                b_index.clone(),
            );
            push_scalar_field(&mut out, div, resolve);
        }
        if a_query != b_query {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "query_hash",
                hash_hex(a_query),
                hash_hex(b_query),
            );
            push_hash_field(&mut out, div, resolve, Some(a_query), Some(b_query))?;
        }
        if a_results != b_results {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "results_hash",
                hash_hex(a_results),
                hash_hex(b_results),
            );
            push_hash_field(&mut out, div, resolve, Some(a_results), Some(b_results))?;
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
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "policy_id",
                a_policy.clone(),
                b_policy.clone(),
            );
            push_scalar_field(&mut out, div, resolve);
        }
        if a_decision != b_decision {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::PermissionGateDecisionDifference,
                "decision",
                a_decision.clone(),
                b_decision.clone(),
            );
            push_scalar_field(&mut out, div, resolve);
        }
        if a_context != b_context {
            let div = divergence(
                a.sequence,
                DiffDivergenceKind::ContentReferenceDifference,
                "context_hash",
                hash_hex(a_context),
                hash_hex(b_context),
            );
            push_hash_field(&mut out, div, resolve, Some(a_context), Some(b_context))?;
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
    let mut out = Vec::new();
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
        let div = divergence(
            a.sequence,
            DiffDivergenceKind::SessionEndDifference,
            "summary_hash",
            opt_hash_hex(a_summary.as_ref()),
            opt_hash_hex(b_summary.as_ref()),
        );
        push_hash_field(
            &mut out,
            div,
            resolve,
            a_summary.as_ref(),
            b_summary.as_ref(),
        )?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use akmon_journal::{HashAlgorithm, MemoryObjectStore, ObjectStore};
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

    fn provider_event(attempts: Vec<AttemptRecord>) -> Event {
        event(
            2,
            EventKind::ProviderCall {
                provider_id: "otel".to_owned(),
                attempts,
                stream_hash: None,
            },
        )
    }

    #[test]
    fn t_provider_call_request_hash_difference_is_reported() {
        // Same output, different request (e.g. OTEL system instructions live only in request_hash).
        let mut a_attempt = attempt(1, Some(hash(7)));
        let mut b_attempt = attempt(1, Some(hash(7)));
        a_attempt.request_hash = hash(40);
        b_attempt.request_hash = hash(41);
        let diffs = compare_provider_call(
            &provider_event(vec![a_attempt]),
            &provider_event(vec![b_attempt]),
            None,
        )
        .expect("compare");
        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].kind,
            DiffDivergenceKind::ProviderCallRequestDifference
        );
        assert_eq!(diffs[0].field.as_deref(), Some("attempts[0].request_hash"));
        assert_eq!(diffs[0].expected, hash(40).to_hex());
        assert_eq!(diffs[0].actual, hash(41).to_hex());
    }

    #[test]
    fn t_provider_call_request_hash_resolves_bytes() {
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sb = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut a_attempt = attempt(1, Some(hash(7)));
        let mut b_attempt = attempt(1, Some(hash(7)));
        a_attempt.request_hash = sa.put(b"system: be terse").expect("put");
        b_attempt.request_hash = sb.put(b"system: be verbose").expect("put");
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sb.as_ref(),
        };
        let diffs = compare_provider_call(
            &provider_event(vec![a_attempt]),
            &provider_event(vec![b_attempt]),
            Some(ctx),
        )
        .expect("compare");
        assert_eq!(diffs.len(), 1);
        let resolved = diffs[0].resolved.as_ref().expect("resolved");
        assert_eq!(resolved.a_preview.as_deref(), Some("system: be terse"));
        assert!(!resolved.bytes_match);
    }

    #[test]
    fn t_provider_call_attempt_count_difference_is_reported() {
        // A retried call and a single-shot call with the same final response must not match.
        let mut retry = attempt(1, None);
        retry.status = AttemptStatus::RateLimited;
        retry.error_message = Some("429".to_owned());
        let a = provider_event(vec![retry, attempt(2, Some(hash(7)))]);
        let mut only = attempt(1, Some(hash(7)));
        only.request_hash = hash(12);
        let b = provider_event(vec![only]);
        let diffs = compare_provider_call(&a, &b, None).expect("compare");
        let kinds: Vec<_> = diffs.iter().map(|d| d.kind.clone()).collect();
        assert_eq!(
            diffs[0].kind,
            DiffDivergenceKind::ProviderCallAttemptCountDifference
        );
        assert_eq!(diffs[0].field.as_deref(), Some("attempts.len"));
        assert_eq!(diffs[0].expected, "2");
        assert_eq!(diffs[0].actual, "1");
        assert!(kinds.contains(&DiffDivergenceKind::ProviderCallAttemptStatusDifference));
        assert!(kinds.contains(&DiffDivergenceKind::ProviderCallAttemptErrorDifference));
        assert!(!kinds.contains(&DiffDivergenceKind::ProviderCallResponseDifference));
    }

    #[test]
    fn t_provider_call_status_and_error_difference_is_reported() {
        // Failed vs succeeded call with identical response hash (OTEL always sets response_hash).
        let ok = attempt(1, Some(hash(7)));
        let mut failed = attempt(1, Some(hash(7)));
        failed.status = AttemptStatus::Other("rate_limit".to_owned());
        failed.error_message = Some("quota exceeded".to_owned());
        let sa = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let ctx = ResolveContext {
            store_a: sa.as_ref(),
            store_b: sa.as_ref(),
        };
        let diffs = compare_provider_call(
            &provider_event(vec![ok]),
            &provider_event(vec![failed]),
            Some(ctx),
        )
        .expect("compare");
        assert_eq!(diffs.len(), 2);
        assert_eq!(
            diffs[0].kind,
            DiffDivergenceKind::ProviderCallAttemptStatusDifference
        );
        assert_eq!(diffs[0].field.as_deref(), Some("attempts[0].status"));
        assert_eq!(diffs[0].expected, "success");
        assert_eq!(diffs[0].actual, "other:rate_limit");
        assert_eq!(
            diffs[0].resolved_skip_reason.as_deref(),
            Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
        );
        assert_eq!(
            diffs[1].kind,
            DiffDivergenceKind::ProviderCallAttemptErrorDifference
        );
        assert_eq!(diffs[1].field.as_deref(), Some("attempts[0].error_message"));
        assert_eq!(diffs[1].expected, "none");
        assert_eq!(diffs[1].actual, "quota exceeded");
    }
}
