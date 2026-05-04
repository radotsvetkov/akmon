//! Session event types and wire hashing behavior.

use crate::error::{JournalError, Result};
use crate::hash::{Hash, HashAlgorithm, WireHash, digest_bytes};
use serde::{Deserialize, Serialize};

/// One append-only merkle graph event in a journal session.
///
/// Storage contract:
/// - Internally persisted in redb using postcard.
/// - Hashed for AGEF compatibility over canonical CBOR bytes where all `Hash` fields
///   are represented as `WireHash` values and `emitted_at` is encoded as CBOR tag 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Hashes of parent events.
    pub parents: Vec<Hash>,
    /// Event payload kind.
    pub kind: EventKind,
    /// UTC event emission timestamp.
    pub emitted_at: time::OffsetDateTime,
    /// Monotonic per-session sequence, starting at 0.
    pub sequence: u64,
}

impl Event {
    /// Computes the event content hash over canonical CBOR bytes.
    pub fn content_hash(&self, algorithm: HashAlgorithm) -> Result<Hash> {
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&WireEvent::from(self), &mut encoded)
            .map_err(|err| JournalError::Cbor(err.to_string()))?;
        Ok(digest_bytes(algorithm, &encoded))
    }

    /// Encodes this event into the wire shape used for AGEF hashing and export.
    #[cfg(test)]
    pub(crate) fn to_wire_cbor_bytes(&self) -> Result<Vec<u8>> {
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&WireEvent::from(self), &mut encoded)
            .map_err(|err| JournalError::Cbor(err.to_string()))?;
        Ok(encoded)
    }
}

/// The event payload variants for AGEF v0.1 session graphs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// Start of session lifecycle.
    SessionStart { cwd_hash: Hash, config_hash: Hash },
    /// User prompt turn.
    UserTurn { prompt_hash: Hash },
    /// Provider call with all attempts and optional stream transcript pointer.
    ProviderCall {
        provider_id: String,
        attempts: Vec<AttemptRecord>,
        stream_hash: Option<Hash>,
    },
    /// Tool execution call and output references.
    ToolCall {
        tool_id: String,
        input_hash: Hash,
        output_hash: Hash,
        side_effects_hash: Option<Hash>,
    },
    /// Retrieval invocation record.
    RetrievalCall {
        index_id: String,
        query_hash: Hash,
        results_hash: Hash,
    },
    /// Permission decision checkpoint.
    ///
    /// Recommended values for `decision`: "allowed", "denied", "deferred".
    PermissionGate {
        policy_id: String,
        decision: String,
        context_hash: Hash,
    },
    /// Assistant response turn.
    AssistantTurn {
        message_hash: Hash,
        tool_calls_hash: Option<Hash>,
    },
    /// End of session lifecycle.
    SessionEnd { summary_hash: Option<Hash> },
}

/// Content-addressed object hashes referenced by `kind` (verification, bundle export).
#[must_use]
pub fn referenced_object_hashes_for_kind(kind: &EventKind) -> Vec<Hash> {
    let mut hashes = Vec::new();
    match kind {
        EventKind::SessionStart {
            cwd_hash,
            config_hash,
        } => {
            hashes.push(cwd_hash.clone());
            hashes.push(config_hash.clone());
        }
        EventKind::UserTurn { prompt_hash } => hashes.push(prompt_hash.clone()),
        EventKind::ProviderCall {
            attempts,
            stream_hash,
            ..
        } => {
            for attempt in attempts {
                hashes.push(attempt.request_hash.clone());
                if let Some(response) = attempt.response_hash.as_ref() {
                    hashes.push(response.clone());
                }
                if let Some(stream) = attempt.stream_hash.as_ref() {
                    hashes.push(stream.clone());
                }
            }
            if let Some(stream) = stream_hash.as_ref() {
                hashes.push(stream.clone());
            }
        }
        EventKind::ToolCall {
            input_hash,
            output_hash,
            side_effects_hash,
            ..
        } => {
            hashes.push(input_hash.clone());
            hashes.push(output_hash.clone());
            if let Some(side_effects) = side_effects_hash.as_ref() {
                hashes.push(side_effects.clone());
            }
        }
        EventKind::RetrievalCall {
            query_hash,
            results_hash,
            ..
        } => {
            hashes.push(query_hash.clone());
            hashes.push(results_hash.clone());
        }
        EventKind::PermissionGate { context_hash, .. } => hashes.push(context_hash.clone()),
        EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        } => {
            hashes.push(message_hash.clone());
            if let Some(tool_calls) = tool_calls_hash.as_ref() {
                hashes.push(tool_calls.clone());
            }
        }
        EventKind::SessionEnd { summary_hash } => {
            if let Some(summary) = summary_hash.as_ref() {
                hashes.push(summary.clone());
            }
        }
    }
    hashes
}

/// A single provider HTTP/model attempt within one logical provider call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRecord {
    /// 1-indexed attempt number.
    pub attempt_number: u32,
    /// Attempt start timestamp.
    pub started_at: time::OffsetDateTime,
    /// Attempt end timestamp.
    pub ended_at: time::OffsetDateTime,
    /// Attempt completion status.
    pub status: AttemptStatus,
    /// Hash of serialized request payload.
    pub request_hash: Hash,
    /// Hash of response payload if one exists.
    pub response_hash: Option<Hash>,
    /// Hash of stream transcript object if one exists.
    pub stream_hash: Option<Hash>,
    /// Human-readable error message (if failed).
    pub error_message: Option<String>,
}

/// Attempt result classification for ProviderCall retries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptStatus {
    /// Call completed successfully.
    Success,
    /// Backend throttled request.
    RateLimited,
    /// Network transport failure.
    NetworkError,
    /// Upstream service returned server-side error.
    ServerError,
    /// Request was rejected as malformed or invalid.
    ClientError,
    /// Request was cancelled before completion.
    Cancelled,
    /// Other status not represented above.
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WireEvent {
    parents: Vec<WireHash>,
    kind: WireEventKind,
    emitted_at: ciborium::tag::Required<i64, 1>,
    sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum WireEventKind {
    SessionStart {
        cwd_hash: WireHash,
        config_hash: WireHash,
    },
    UserTurn {
        prompt_hash: WireHash,
    },
    ProviderCall {
        provider_id: String,
        attempts: Vec<WireAttemptRecord>,
        stream_hash: Option<WireHash>,
    },
    ToolCall {
        tool_id: String,
        input_hash: WireHash,
        output_hash: WireHash,
        side_effects_hash: Option<WireHash>,
    },
    RetrievalCall {
        index_id: String,
        query_hash: WireHash,
        results_hash: WireHash,
    },
    PermissionGate {
        policy_id: String,
        decision: String,
        context_hash: WireHash,
    },
    AssistantTurn {
        message_hash: WireHash,
        tool_calls_hash: Option<WireHash>,
    },
    SessionEnd {
        summary_hash: Option<WireHash>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireAttemptRecord {
    attempt_number: u32,
    started_at: ciborium::tag::Required<i64, 1>,
    ended_at: ciborium::tag::Required<i64, 1>,
    status: AttemptStatus,
    request_hash: WireHash,
    response_hash: Option<WireHash>,
    stream_hash: Option<WireHash>,
    error_message: Option<String>,
}

impl From<&Event> for WireEvent {
    fn from(value: &Event) -> Self {
        Self {
            parents: value.parents.iter().map(Hash::to_wire).collect(),
            kind: WireEventKind::from(&value.kind),
            emitted_at: ciborium::tag::Required(value.emitted_at.unix_timestamp()),
            sequence: value.sequence,
        }
    }
}

impl From<&EventKind> for WireEventKind {
    fn from(value: &EventKind) -> Self {
        match value {
            EventKind::SessionStart {
                cwd_hash,
                config_hash,
            } => Self::SessionStart {
                cwd_hash: cwd_hash.to_wire(),
                config_hash: config_hash.to_wire(),
            },
            EventKind::UserTurn { prompt_hash } => Self::UserTurn {
                prompt_hash: prompt_hash.to_wire(),
            },
            EventKind::ProviderCall {
                provider_id,
                attempts,
                stream_hash,
            } => Self::ProviderCall {
                provider_id: provider_id.clone(),
                attempts: attempts.iter().map(WireAttemptRecord::from).collect(),
                stream_hash: stream_hash.as_ref().map(Hash::to_wire),
            },
            EventKind::ToolCall {
                tool_id,
                input_hash,
                output_hash,
                side_effects_hash,
            } => Self::ToolCall {
                tool_id: tool_id.clone(),
                input_hash: input_hash.to_wire(),
                output_hash: output_hash.to_wire(),
                side_effects_hash: side_effects_hash.as_ref().map(Hash::to_wire),
            },
            EventKind::RetrievalCall {
                index_id,
                query_hash,
                results_hash,
            } => Self::RetrievalCall {
                index_id: index_id.clone(),
                query_hash: query_hash.to_wire(),
                results_hash: results_hash.to_wire(),
            },
            EventKind::PermissionGate {
                policy_id,
                decision,
                context_hash,
            } => Self::PermissionGate {
                policy_id: policy_id.clone(),
                decision: decision.clone(),
                context_hash: context_hash.to_wire(),
            },
            EventKind::AssistantTurn {
                message_hash,
                tool_calls_hash,
            } => Self::AssistantTurn {
                message_hash: message_hash.to_wire(),
                tool_calls_hash: tool_calls_hash.as_ref().map(Hash::to_wire),
            },
            EventKind::SessionEnd { summary_hash } => Self::SessionEnd {
                summary_hash: summary_hash.as_ref().map(Hash::to_wire),
            },
        }
    }
}

impl From<&AttemptRecord> for WireAttemptRecord {
    fn from(value: &AttemptRecord) -> Self {
        Self {
            attempt_number: value.attempt_number,
            started_at: ciborium::tag::Required(value.started_at.unix_timestamp()),
            ended_at: ciborium::tag::Required(value.ended_at.unix_timestamp()),
            status: value.status.clone(),
            request_hash: value.request_hash.to_wire(),
            response_hash: value.response_hash.as_ref().map(Hash::to_wire),
            stream_hash: value.stream_hash.as_ref().map(Hash::to_wire),
            error_message: value.error_message.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    fn h(algorithm: HashAlgorithm, value: u8) -> Hash {
        Hash::from_bytes(algorithm, [value; 32])
    }

    fn t(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).unwrap_or_else(|_| unreachable!())
    }

    fn sample_session_start_event() -> Event {
        Event {
            parents: vec![
                h(HashAlgorithm::Sha256, 0x10),
                h(HashAlgorithm::Sha256, 0x11),
            ],
            kind: EventKind::SessionStart {
                cwd_hash: h(HashAlgorithm::Sha256, 0x12),
                config_hash: h(HashAlgorithm::Sha256, 0x13),
            },
            emitted_at: t(1_711_111_111),
            sequence: 42,
        }
    }

    fn sample_provider_call_event() -> Event {
        Event {
            parents: vec![h(HashAlgorithm::Sha256, 0x21)],
            kind: EventKind::ProviderCall {
                provider_id: "anthropic".to_owned(),
                attempts: vec![
                    AttemptRecord {
                        attempt_number: 1,
                        started_at: t(1_711_111_120),
                        ended_at: t(1_711_111_121),
                        status: AttemptStatus::RateLimited,
                        request_hash: h(HashAlgorithm::Sha256, 0x22),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some("429".to_owned()),
                    },
                    AttemptRecord {
                        attempt_number: 2,
                        started_at: t(1_711_111_122),
                        ended_at: t(1_711_111_125),
                        status: AttemptStatus::Success,
                        request_hash: h(HashAlgorithm::Sha256, 0x23),
                        response_hash: Some(h(HashAlgorithm::Sha256, 0x24)),
                        stream_hash: Some(h(HashAlgorithm::Sha256, 0x25)),
                        error_message: None,
                    },
                ],
                stream_hash: Some(h(HashAlgorithm::Sha256, 0x26)),
            },
            emitted_at: t(1_711_111_126),
            sequence: 43,
        }
    }

    #[test]
    fn event_postcard_roundtrip_session_start() {
        let event = sample_session_start_event();
        let encoded = postcard::to_allocvec(&event).unwrap_or_else(|_| unreachable!());
        let decoded: Event = postcard::from_bytes(&encoded).unwrap_or_else(|_| unreachable!());
        assert_eq!(decoded, event);
    }

    #[test]
    fn event_postcard_roundtrip_provider_call_multi_attempt() {
        let event = sample_provider_call_event();
        let encoded = postcard::to_allocvec(&event).unwrap_or_else(|_| unreachable!());
        let decoded: Event = postcard::from_bytes(&encoded).unwrap_or_else(|_| unreachable!());
        assert_eq!(decoded, event);
    }

    #[test]
    fn event_wire_cbor_encoding_is_canonical_for_equal_values() {
        let event_one = sample_provider_call_event();
        let event_two = sample_provider_call_event();
        let bytes_one = event_one
            .to_wire_cbor_bytes()
            .unwrap_or_else(|_| unreachable!());
        let bytes_two = event_two
            .to_wire_cbor_bytes()
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(bytes_one, bytes_two);
    }

    #[test]
    fn content_hash_is_deterministic_and_algorithm_distinct() {
        let event_a1 = sample_provider_call_event();
        let event_a2 = sample_provider_call_event();
        let mut event_b = sample_provider_call_event();
        event_b.sequence += 1;

        let h1 = event_a1
            .content_hash(HashAlgorithm::Sha256)
            .unwrap_or_else(|_| unreachable!());
        let h2 = event_a2
            .content_hash(HashAlgorithm::Sha256)
            .unwrap_or_else(|_| unreachable!());
        let h3 = event_b
            .content_hash(HashAlgorithm::Sha256)
            .unwrap_or_else(|_| unreachable!());
        let h4 = event_a1
            .content_hash(HashAlgorithm::Blake3)
            .unwrap_or_else(|_| unreachable!());

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_ne!(h1.algorithm, h4.algorithm);
        assert_ne!(h1.bytes, h4.bytes);
    }

    #[test]
    fn wire_cbor_contains_tag_1_epoch_seconds_for_timestamp() {
        let event = sample_session_start_event();
        let bytes = event
            .to_wire_cbor_bytes()
            .unwrap_or_else(|_| unreachable!());

        let expected_epoch = event.emitted_at.unix_timestamp();
        let mut epoch_encoded = Vec::new();
        ciborium::ser::into_writer(&expected_epoch, &mut epoch_encoded)
            .unwrap_or_else(|_| unreachable!());
        let mut needle = vec![0xC1];
        needle.extend_from_slice(&epoch_encoded);

        assert!(bytes.windows(needle.len()).any(|w| w == needle.as_slice()));
    }

    #[test]
    fn attempt_status_all_variants_roundtrip_postcard() {
        let variants = [
            AttemptStatus::Success,
            AttemptStatus::RateLimited,
            AttemptStatus::NetworkError,
            AttemptStatus::ServerError,
            AttemptStatus::ClientError,
            AttemptStatus::Cancelled,
            AttemptStatus::Other("custom-status".to_owned()),
        ];

        for status in variants {
            let encoded = postcard::to_allocvec(&status).unwrap_or_else(|_| unreachable!());
            let decoded: AttemptStatus =
                postcard::from_bytes(&encoded).unwrap_or_else(|_| unreachable!());
            assert_eq!(decoded, status);
        }
    }

    #[test]
    fn event_kind_all_variants_roundtrip_and_distinct_postcard() {
        let variants = [
            EventKind::SessionStart {
                cwd_hash: h(HashAlgorithm::Sha256, 0x01),
                config_hash: h(HashAlgorithm::Sha256, 0x02),
            },
            EventKind::UserTurn {
                prompt_hash: h(HashAlgorithm::Sha256, 0x03),
            },
            EventKind::ProviderCall {
                provider_id: "p".to_owned(),
                attempts: vec![AttemptRecord {
                    attempt_number: 1,
                    started_at: t(10),
                    ended_at: t(11),
                    status: AttemptStatus::Success,
                    request_hash: h(HashAlgorithm::Sha256, 0x04),
                    response_hash: Some(h(HashAlgorithm::Sha256, 0x05)),
                    stream_hash: None,
                    error_message: None,
                }],
                stream_hash: Some(h(HashAlgorithm::Sha256, 0x06)),
            },
            EventKind::ToolCall {
                tool_id: "tool".to_owned(),
                input_hash: h(HashAlgorithm::Sha256, 0x07),
                output_hash: h(HashAlgorithm::Sha256, 0x08),
                side_effects_hash: Some(h(HashAlgorithm::Sha256, 0x09)),
            },
            EventKind::RetrievalCall {
                index_id: "idx".to_owned(),
                query_hash: h(HashAlgorithm::Sha256, 0x0A),
                results_hash: h(HashAlgorithm::Sha256, 0x0B),
            },
            EventKind::PermissionGate {
                policy_id: "policy".to_owned(),
                decision: "allowed".to_owned(),
                context_hash: h(HashAlgorithm::Sha256, 0x0C),
            },
            EventKind::AssistantTurn {
                message_hash: h(HashAlgorithm::Sha256, 0x0D),
                tool_calls_hash: Some(h(HashAlgorithm::Sha256, 0x0E)),
            },
            EventKind::SessionEnd {
                summary_hash: Some(h(HashAlgorithm::Sha256, 0x0F)),
            },
        ];

        let mut encodings = Vec::new();
        for variant in variants {
            let encoded = postcard::to_allocvec(&variant).unwrap_or_else(|_| unreachable!());
            let decoded: EventKind =
                postcard::from_bytes(&encoded).unwrap_or_else(|_| unreachable!());
            assert_eq!(decoded, variant);
            encodings.push(encoded);
        }

        for i in 0..encodings.len() {
            for j in (i + 1)..encodings.len() {
                assert_ne!(encodings[i], encodings[j]);
            }
        }
    }

    #[test]
    fn wire_cbor_provider_call_contains_no_algorithm_strings() {
        let event = sample_provider_call_event();
        let bytes = event
            .to_wire_cbor_bytes()
            .unwrap_or_else(|_| unreachable!());
        let hay = bytes.as_slice();
        assert!(!hay.windows("sha256".len()).any(|w| w == b"sha256"));
        assert!(!hay.windows("blake3".len()).any(|w| w == b"blake3"));
    }

    #[test]
    fn print_wire_cbor_bytes_for_manual_inspection() {
        let event = sample_provider_call_event();
        let bytes = event
            .to_wire_cbor_bytes()
            .unwrap_or_else(|_| unreachable!());
        println!("{:02x?}", bytes);
        assert!(!bytes.is_empty());
    }
}
