use serde::{Deserialize, Serialize};

use crate::{ReplayDivergence, ReplayError, ReplayRunOutput};

const AGEF_VERSION: &str = "0.1";

/// Final replay report schema emitted by Item 5.2 orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReportV1 {
    /// Akmon crate version producing this report.
    pub akmon_version: String,
    /// AGEF format version targeted by replay evidence.
    pub agef_version: String,
    /// Source session UUID (hyphenated).
    pub source_session_id: String,
    /// Source session head hash (hex).
    pub source_head: String,
    /// Replay session UUID when replay output is persisted.
    pub replay_session_id: Option<String>,
    /// Replay mode string (`default` or `strict`).
    pub mode: String,
    /// Number of lockstep-compared events.
    pub events_compared: u64,
    /// Source session event count.
    pub source_event_count: u64,
    /// Replay session event count.
    pub replay_event_count: u64,
    /// Divergences detected during primitive playback execution.
    pub primitive_divergence_count: u64,
    /// Divergences detected during engine-level history comparison.
    pub engine_divergence_count: u64,
    /// Ordered divergence list for downstream rendering.
    pub divergences: Vec<ReplayDivergence>,
    /// Whether replay passed without divergences.
    pub passed: bool,
}

/// Serializable replay infrastructure/setup/runtime failure envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayInfraError {
    /// Akmon crate version producing this error payload.
    pub akmon_version: String,
    /// Human-readable error string.
    pub error: String,
    /// Stable error category string.
    pub category: String,
    /// Optional source session UUID when known.
    pub source_session_id: Option<String>,
    /// Missing provider id context.
    pub missing_provider_id: Option<String>,
    /// Missing tool id context.
    pub missing_tool_id: Option<String>,
    /// Missing object hash context.
    pub missing_object_hash: Option<String>,
}

/// Assembles final replay report from run output and engine-level comparison divergences.
pub fn assemble_report(
    output: ReplayRunOutput,
    engine_divergences: Vec<ReplayDivergence>,
) -> ReplayReportV1 {
    let primitive_divergence_count = output.divergences.len() as u64;
    let engine_divergence_count = engine_divergences.len() as u64;
    let mut divergences = Vec::with_capacity(output.divergences.len() + engine_divergences.len());
    divergences.extend(output.divergences);
    divergences.extend(engine_divergences);
    divergences.sort_by_key(|d| d.event_seq.unwrap_or(u64::MAX));
    let source_event_count = output.source_history.len() as u64;
    let replay_event_count = output.replay_history.len() as u64;
    let events_compared = source_event_count.min(replay_event_count);
    let source_head = output
        .source_history
        .last()
        .map(|(h, _)| h.to_hex())
        .unwrap_or_default();
    let replay_session_id = output
        .replay_persisted
        .then(|| output.replay_session_id.to_string());
    let passed = divergences.is_empty();
    ReplayReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: AGEF_VERSION.to_owned(),
        source_session_id: output.source_session_id.to_string(),
        source_head,
        replay_session_id,
        mode: output.mode.to_string(),
        events_compared,
        source_event_count,
        replay_event_count,
        primitive_divergence_count,
        engine_divergence_count,
        divergences,
        passed,
    }
}

impl ReplayInfraError {
    /// Builds serializable infra error payload from replay error.
    pub fn from_replay_error(err: &ReplayError, source_session_id: Option<String>) -> Self {
        let (category, missing_provider_id, missing_tool_id, missing_object_hash) = match err {
            ReplayError::EmptySource => ("empty_source", None, None, None),
            ReplayError::NoMatchingCalls(_) => ("no_matching_calls", None, None, None),
            ReplayError::MissingSourceObject(hash) => {
                ("missing_source_object", None, None, Some(hash.to_hex()))
            }
            ReplayError::MissingToolForReplay { tool_id } => {
                ("missing_tool_for_replay", None, Some(tool_id.clone()), None)
            }
            ReplayError::MissingProviderForReplay { provider_id } => (
                "missing_provider_for_replay",
                Some(provider_id.clone()),
                None,
                None,
            ),
            ReplayError::MalformedSourceEvent { .. } => {
                ("malformed_source_event", None, None, None)
            }
            ReplayError::MalformedSourceConfig { .. } => {
                ("malformed_source_config", None, None, None)
            }
            ReplayError::StoreReadFailed { .. } => ("store_read_failed", None, None, None),
            ReplayError::UnsupportedProviderMultiplicity { .. } => {
                ("unsupported_provider_multiplicity", None, None, None)
            }
            ReplayError::SessionRunFailed { .. } => ("session_run_failed", None, None, None),
            ReplayError::ReplaySessionMalformed { .. } => {
                ("replay_session_malformed", None, None, None)
            }
            ReplayError::PersistConfigInvalid { .. } => {
                ("persist_config_invalid", None, None, None)
            }
            ReplayError::PersistJournalNotWritable { .. } => {
                ("persist_journal_not_writable", None, None, None)
            }
        };
        Self {
            akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
            error: err.to_string(),
            category: category.to_owned(),
            source_session_id,
            missing_provider_id,
            missing_tool_id,
            missing_object_hash,
        }
    }
}

#[cfg(test)]
mod tests {
    use akmon_journal::{Event, EventKind, Hash, HashAlgorithm};
    use time::OffsetDateTime;
    use uuid::Uuid;

    use super::*;
    use crate::{ReplayDivergenceKind, ReplayMode};

    fn hash_of(byte: u8) -> Hash {
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

    fn output_with(primitive: Vec<ReplayDivergence>) -> ReplayRunOutput {
        ReplayRunOutput {
            source_session_id: Uuid::new_v4(),
            replay_session_id: Uuid::new_v4(),
            mode: ReplayMode::Default,
            source_history: vec![
                (
                    hash_of(1),
                    event(
                        0,
                        EventKind::SessionStart {
                            cwd_hash: hash_of(2),
                            config_hash: hash_of(3),
                        },
                    ),
                ),
                (
                    hash_of(4),
                    event(1, EventKind::SessionEnd { summary_hash: None }),
                ),
            ],
            replay_history: vec![
                (
                    hash_of(5),
                    event(
                        0,
                        EventKind::SessionStart {
                            cwd_hash: hash_of(2),
                            config_hash: hash_of(3),
                        },
                    ),
                ),
                (
                    hash_of(6),
                    event(1, EventKind::SessionEnd { summary_hash: None }),
                ),
            ],
            divergences: primitive,
            replay_persisted: false,
        }
    }

    #[test]
    fn t_assemble_report_clean_session_passed_true() {
        let report = assemble_report(output_with(Vec::new()), Vec::new());
        assert!(report.passed);
        assert_eq!(report.divergences.len(), 0);
        assert_eq!(report.source_event_count, 2);
        assert_eq!(report.replay_event_count, 2);
        assert_eq!(report.events_compared, 2);
    }

    #[test]
    fn t_assemble_report_with_divergences_passed_false() {
        let primitive = vec![ReplayDivergence {
            event_seq: Some(1),
            kind: ReplayDivergenceKind::ToolCallUnexpected,
            expected: "none".to_owned(),
            actual: "tool call".to_owned(),
        }];
        let engine = vec![ReplayDivergence {
            event_seq: Some(2),
            kind: ReplayDivergenceKind::EventKindMismatch,
            expected: "UserTurn".to_owned(),
            actual: "ToolCall".to_owned(),
        }];
        let report = assemble_report(output_with(primitive), engine);
        assert!(!report.passed);
        assert_eq!(report.divergences.len(), 2);
    }

    #[test]
    fn t_assemble_report_counts_separate_primitive_and_engine() {
        let primitive = vec![ReplayDivergence {
            event_seq: Some(1),
            kind: ReplayDivergenceKind::ToolInputMismatch,
            expected: "x".to_owned(),
            actual: "y".to_owned(),
        }];
        let engine = vec![ReplayDivergence {
            event_seq: Some(3),
            kind: ReplayDivergenceKind::AssistantContentMismatch,
            expected: "a".to_owned(),
            actual: "b".to_owned(),
        }];
        let report = assemble_report(output_with(primitive), engine);
        assert_eq!(report.primitive_divergence_count, 1);
        assert_eq!(report.engine_divergence_count, 1);
    }

    #[test]
    fn t_assemble_report_divergences_ordered_by_event_seq() {
        let primitive = vec![ReplayDivergence {
            event_seq: Some(8),
            kind: ReplayDivergenceKind::ToolInputMismatch,
            expected: "x".to_owned(),
            actual: "y".to_owned(),
        }];
        let engine = vec![
            ReplayDivergence {
                event_seq: None,
                kind: ReplayDivergenceKind::EventCountMismatch,
                expected: "2".to_owned(),
                actual: "3".to_owned(),
            },
            ReplayDivergence {
                event_seq: Some(3),
                kind: ReplayDivergenceKind::EventKindMismatch,
                expected: "A".to_owned(),
                actual: "B".to_owned(),
            },
        ];
        let report = assemble_report(output_with(primitive), engine);
        assert_eq!(report.divergences[0].event_seq, Some(3));
        assert_eq!(report.divergences[1].event_seq, Some(8));
        assert_eq!(report.divergences[2].event_seq, None);
    }

    #[test]
    fn t_replay_report_v1_serializes_to_json() {
        let report = assemble_report(output_with(Vec::new()), Vec::new());
        let json = serde_json::to_value(&report).expect("serialize");
        assert!(json.get("akmon_version").is_some());
        assert_eq!(json.get("mode").and_then(|v| v.as_str()), Some("default"));
        assert!(json.get("source_head").is_some());
    }

    #[test]
    fn t_replay_report_v1_round_trip() {
        let report = assemble_report(output_with(Vec::new()), Vec::new());
        let json = serde_json::to_string(&report).expect("serialize");
        let decoded: ReplayReportV1 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, report);
    }
}
