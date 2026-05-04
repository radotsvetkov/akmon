//! `events.bin` framing primitives.

use crate::BundleError;
use akmon_journal::{AttemptRecord, AttemptStatus, Event, EventKind, Hash, HashAlgorithm};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Default safety limit for one event frame length in bytes.
pub const DEFAULT_MAX_EVENT_FRAME_LEN: u32 = 1_048_576;

/// Length-delimited `events.bin` writer.
pub struct EventsWriter<W: Write> {
    inner: W,
}

impl<W: Write> EventsWriter<W> {
    /// Creates a new writer using SHA-256 hash interpretation for wire hash fields.
    pub fn new(writer: W) -> Self {
        Self { inner: writer }
    }

    /// Creates a writer with explicit hash algorithm for wire hash interpretation.
    pub fn with_hash_algorithm(writer: W, _algorithm: HashAlgorithm) -> Self {
        Self { inner: writer }
    }

    /// Writes one event frame (`u32` big-endian length + canonical CBOR payload).
    pub fn write_event(&mut self, event: &Event) -> Result<(), BundleError> {
        let wire = WireEvent::from_event(event);
        let mut payload = Vec::new();
        ciborium::ser::into_writer(&wire, &mut payload)
            .map_err(|err| BundleError::MalformedFraming(format!("CBOR encode failed: {err}")))?;
        let len = u32::try_from(payload.len()).map_err(|_| {
            BundleError::MalformedFraming("event payload length exceeds u32::MAX".to_owned())
        })?;
        self.inner.write_all(&len.to_be_bytes())?;
        self.inner.write_all(&payload)?;
        Ok(())
    }

    /// Finishes writing and returns the wrapped writer.
    pub fn finish(self) -> Result<W, BundleError> {
        Ok(self.inner)
    }
}

/// Length-delimited `events.bin` reader.
pub struct EventsReader<R: Read> {
    inner: R,
    max_frame_len: u32,
    algorithm: HashAlgorithm,
}

impl<R: Read> EventsReader<R> {
    /// Creates a new reader using SHA-256 hash interpretation for wire hash fields.
    pub fn new(reader: R) -> Self {
        Self {
            inner: reader,
            max_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
            algorithm: HashAlgorithm::Sha256,
        }
    }

    /// Creates a new reader with explicit maximum frame length.
    pub fn with_max_frame_len(reader: R, max_frame_len: u32) -> Self {
        Self {
            inner: reader,
            max_frame_len,
            algorithm: HashAlgorithm::Sha256,
        }
    }

    /// Creates a new reader with explicit hash algorithm.
    pub fn with_hash_algorithm(reader: R, algorithm: HashAlgorithm) -> Self {
        Self {
            inner: reader,
            max_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
            algorithm,
        }
    }

    /// Creates a reader with explicit hash algorithm and frame length limit.
    pub fn with_hash_algorithm_and_max_frame_len(
        reader: R,
        algorithm: HashAlgorithm,
        max_frame_len: u32,
    ) -> Self {
        Self {
            inner: reader,
            max_frame_len,
            algorithm,
        }
    }

    /// Reads one framed event, returning `Ok(None)` at clean EOF.
    pub fn read_event(&mut self) -> Result<Option<Event>, BundleError> {
        let mut len_buf = [0_u8; 4];
        let mut got = 0usize;
        while got < len_buf.len() {
            let read = self.inner.read(&mut len_buf[got..])?;
            if read == 0 {
                if got == 0 {
                    return Ok(None);
                }
                return Err(BundleError::MalformedFraming(
                    "truncated length prefix".to_owned(),
                ));
            }
            got += read;
        }
        let frame_len = u32::from_be_bytes(len_buf);
        if frame_len > self.max_frame_len {
            return Err(BundleError::FrameTooLarge(frame_len));
        }

        let mut payload = vec![0_u8; frame_len as usize];
        self.inner.read_exact(&mut payload).map_err(|_| {
            BundleError::MalformedFraming(format!("truncated frame: expected {frame_len} bytes"))
        })?;

        let wire: WireEvent = ciborium::de::from_reader(payload.as_slice())
            .map_err(|err| map_wire_decode_error(err.to_string()))?;

        // TODO(performance): Investigate ciborium strict-canonical decode mode to avoid
        // double encoding. The current re-encode-and-compare approach is correct but ~2x
        // slower per event. Acceptable for verify and inspect use cases; may need
        // optimization for replay's per-event hot path.
        let mut canonical = Vec::new();
        ciborium::ser::into_writer(&wire, &mut canonical).map_err(|err| {
            BundleError::MalformedFraming(format!("CBOR re-encode failed: {err}"))
        })?;
        if canonical != payload {
            return Err(BundleError::NonCanonicalCbor);
        }
        Ok(Some(wire.into_event(self.algorithm)))
    }
}

fn map_wire_decode_error(msg: String) -> BundleError {
    if msg.contains("unknown variant") && msg.contains("AttemptStatus") {
        return BundleError::UnknownAttemptStatus(msg);
    }
    if msg.contains("unknown variant") {
        return BundleError::UnknownEventKind(msg);
    }
    BundleError::MalformedFraming(format!("CBOR decode failed: {msg}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireEvent {
    parents: Vec<[u8; 32]>,
    kind: WireEventKind,
    emitted_at: ciborium::tag::Required<i64, 1>,
    sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum WireEventKind {
    SessionStart {
        cwd_hash: [u8; 32],
        config_hash: [u8; 32],
    },
    UserTurn {
        prompt_hash: [u8; 32],
    },
    ProviderCall {
        provider_id: String,
        attempts: Vec<WireAttemptRecord>,
        stream_hash: Option<[u8; 32]>,
    },
    ToolCall {
        tool_id: String,
        input_hash: [u8; 32],
        output_hash: [u8; 32],
        side_effects_hash: Option<[u8; 32]>,
    },
    RetrievalCall {
        index_id: String,
        query_hash: [u8; 32],
        results_hash: [u8; 32],
    },
    PermissionGate {
        policy_id: String,
        decision: String,
        context_hash: [u8; 32],
    },
    AssistantTurn {
        message_hash: [u8; 32],
        tool_calls_hash: Option<[u8; 32]>,
    },
    SessionEnd {
        summary_hash: Option<[u8; 32]>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireAttemptRecord {
    attempt_number: u32,
    started_at: ciborium::tag::Required<i64, 1>,
    ended_at: ciborium::tag::Required<i64, 1>,
    status: AttemptStatus,
    request_hash: [u8; 32],
    response_hash: Option<[u8; 32]>,
    stream_hash: Option<[u8; 32]>,
    error_message: Option<String>,
}

impl WireEvent {
    fn from_event(event: &Event) -> Self {
        Self {
            parents: event.parents.iter().map(|h| h.bytes).collect(),
            kind: WireEventKind::from_kind(&event.kind),
            emitted_at: ciborium::tag::Required(event.emitted_at.unix_timestamp()),
            sequence: event.sequence,
        }
    }

    fn into_event(self, algorithm: HashAlgorithm) -> Event {
        Event {
            parents: self
                .parents
                .into_iter()
                .map(|b| Hash::from_bytes(algorithm, b))
                .collect(),
            kind: self.kind.into_kind(algorithm),
            emitted_at: time::OffsetDateTime::from_unix_timestamp(self.emitted_at.0)
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            sequence: self.sequence,
        }
    }
}

impl WireEventKind {
    fn from_kind(kind: &EventKind) -> Self {
        match kind {
            EventKind::SessionStart {
                cwd_hash,
                config_hash,
            } => Self::SessionStart {
                cwd_hash: cwd_hash.bytes,
                config_hash: config_hash.bytes,
            },
            EventKind::UserTurn { prompt_hash } => Self::UserTurn {
                prompt_hash: prompt_hash.bytes,
            },
            EventKind::ProviderCall {
                provider_id,
                attempts,
                stream_hash,
            } => Self::ProviderCall {
                provider_id: provider_id.clone(),
                attempts: attempts
                    .iter()
                    .map(WireAttemptRecord::from_attempt)
                    .collect(),
                stream_hash: stream_hash.as_ref().map(|h| h.bytes),
            },
            EventKind::ToolCall {
                tool_id,
                input_hash,
                output_hash,
                side_effects_hash,
            } => Self::ToolCall {
                tool_id: tool_id.clone(),
                input_hash: input_hash.bytes,
                output_hash: output_hash.bytes,
                side_effects_hash: side_effects_hash.as_ref().map(|h| h.bytes),
            },
            EventKind::RetrievalCall {
                index_id,
                query_hash,
                results_hash,
            } => Self::RetrievalCall {
                index_id: index_id.clone(),
                query_hash: query_hash.bytes,
                results_hash: results_hash.bytes,
            },
            EventKind::PermissionGate {
                policy_id,
                decision,
                context_hash,
            } => Self::PermissionGate {
                policy_id: policy_id.clone(),
                decision: decision.clone(),
                context_hash: context_hash.bytes,
            },
            EventKind::AssistantTurn {
                message_hash,
                tool_calls_hash,
            } => Self::AssistantTurn {
                message_hash: message_hash.bytes,
                tool_calls_hash: tool_calls_hash.as_ref().map(|h| h.bytes),
            },
            EventKind::SessionEnd { summary_hash } => Self::SessionEnd {
                summary_hash: summary_hash.as_ref().map(|h| h.bytes),
            },
        }
    }

    fn into_kind(self, algorithm: HashAlgorithm) -> EventKind {
        match self {
            WireEventKind::SessionStart {
                cwd_hash,
                config_hash,
            } => EventKind::SessionStart {
                cwd_hash: Hash::from_bytes(algorithm, cwd_hash),
                config_hash: Hash::from_bytes(algorithm, config_hash),
            },
            WireEventKind::UserTurn { prompt_hash } => EventKind::UserTurn {
                prompt_hash: Hash::from_bytes(algorithm, prompt_hash),
            },
            WireEventKind::ProviderCall {
                provider_id,
                attempts,
                stream_hash,
            } => EventKind::ProviderCall {
                provider_id,
                attempts: attempts
                    .into_iter()
                    .map(|a| a.into_attempt(algorithm))
                    .collect(),
                stream_hash: stream_hash.map(|b| Hash::from_bytes(algorithm, b)),
            },
            WireEventKind::ToolCall {
                tool_id,
                input_hash,
                output_hash,
                side_effects_hash,
            } => EventKind::ToolCall {
                tool_id,
                input_hash: Hash::from_bytes(algorithm, input_hash),
                output_hash: Hash::from_bytes(algorithm, output_hash),
                side_effects_hash: side_effects_hash.map(|b| Hash::from_bytes(algorithm, b)),
            },
            WireEventKind::RetrievalCall {
                index_id,
                query_hash,
                results_hash,
            } => EventKind::RetrievalCall {
                index_id,
                query_hash: Hash::from_bytes(algorithm, query_hash),
                results_hash: Hash::from_bytes(algorithm, results_hash),
            },
            WireEventKind::PermissionGate {
                policy_id,
                decision,
                context_hash,
            } => EventKind::PermissionGate {
                policy_id,
                decision,
                context_hash: Hash::from_bytes(algorithm, context_hash),
            },
            WireEventKind::AssistantTurn {
                message_hash,
                tool_calls_hash,
            } => EventKind::AssistantTurn {
                message_hash: Hash::from_bytes(algorithm, message_hash),
                tool_calls_hash: tool_calls_hash.map(|b| Hash::from_bytes(algorithm, b)),
            },
            WireEventKind::SessionEnd { summary_hash } => EventKind::SessionEnd {
                summary_hash: summary_hash.map(|b| Hash::from_bytes(algorithm, b)),
            },
        }
    }
}

impl WireAttemptRecord {
    fn from_attempt(attempt: &AttemptRecord) -> Self {
        Self {
            attempt_number: attempt.attempt_number,
            started_at: ciborium::tag::Required(attempt.started_at.unix_timestamp()),
            ended_at: ciborium::tag::Required(attempt.ended_at.unix_timestamp()),
            status: attempt.status.clone(),
            request_hash: attempt.request_hash.bytes,
            response_hash: attempt.response_hash.as_ref().map(|h| h.bytes),
            stream_hash: attempt.stream_hash.as_ref().map(|h| h.bytes),
            error_message: attempt.error_message.clone(),
        }
    }

    fn into_attempt(self, algorithm: HashAlgorithm) -> AttemptRecord {
        AttemptRecord {
            attempt_number: self.attempt_number,
            started_at: time::OffsetDateTime::from_unix_timestamp(self.started_at.0)
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            ended_at: time::OffsetDateTime::from_unix_timestamp(self.ended_at.0)
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            status: self.status,
            request_hash: Hash::from_bytes(algorithm, self.request_hash),
            response_hash: self.response_hash.map(|b| Hash::from_bytes(algorithm, b)),
            stream_hash: self.stream_hash.map(|b| Hash::from_bytes(algorithm, b)),
            error_message: self.error_message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(seed: u8) -> Hash {
        Hash::from_bytes(HashAlgorithm::Sha256, [seed; 32])
    }

    fn sample_event(seq: u64) -> Event {
        Event {
            parents: if seq == 0 { vec![] } else { vec![hash(0x10)] },
            kind: if seq == 0 {
                EventKind::SessionStart {
                    cwd_hash: hash(0x11),
                    config_hash: hash(0x12),
                }
            } else {
                EventKind::SessionEnd {
                    summary_hash: Some(hash(0x20)),
                }
            },
            emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000 + seq as i64)
                .expect("timestamp"),
            sequence: seq,
        }
    }

    #[test]
    fn t_events_round_trip_single_event() {
        let mut out = Vec::new();
        let mut writer = EventsWriter::new(&mut out);
        let e = sample_event(0);
        writer.write_event(&e).expect("write");
        writer.finish().expect("finish");

        let mut reader = EventsReader::new(out.as_slice());
        let parsed = reader.read_event().expect("read").expect("some");
        assert_eq!(parsed, e);
    }

    #[test]
    fn t_events_round_trip_multiple_events() {
        let mut out = Vec::new();
        let mut writer = EventsWriter::new(&mut out);
        let e0 = sample_event(0);
        let e1 = sample_event(1);
        writer.write_event(&e0).expect("write e0");
        writer.write_event(&e1).expect("write e1");
        writer.finish().expect("finish");

        let mut reader = EventsReader::new(out.as_slice());
        assert_eq!(reader.read_event().expect("e0"), Some(e0));
        assert_eq!(reader.read_event().expect("e1"), Some(e1));
    }

    #[test]
    fn t_events_reader_returns_none_at_eof() {
        let mut reader = EventsReader::new([].as_slice());
        assert!(reader.read_event().expect("eof").is_none());
    }

    #[test]
    fn t_events_reader_rejects_truncated_frame() {
        let bytes = vec![0, 0, 0, 10, 1, 2, 3];
        let mut reader = EventsReader::new(bytes.as_slice());
        assert!(reader.read_event().is_err());
    }

    #[test]
    fn t_events_reader_rejects_oversized_frame() {
        let bytes = vec![0x00, 0x20, 0x00, 0x00];
        let mut reader = EventsReader::with_max_frame_len(bytes.as_slice(), 128);
        let err = reader.read_event().expect_err("must fail");
        assert!(matches!(err, BundleError::FrameTooLarge(_)));
    }

    #[test]
    fn t_events_reader_rejects_non_canonical_cbor() {
        let event = sample_event(0);
        let wire = WireEvent::from_event(&event);
        let mut canonical = Vec::new();
        ciborium::ser::into_writer(&wire, &mut canonical).expect("encode");
        assert_eq!(canonical.first().copied(), Some(0xA4));
        let mut non_canonical = Vec::with_capacity(canonical.len() + 1);
        non_canonical.push(0xB8);
        non_canonical.push(0x04);
        non_canonical.extend_from_slice(&canonical[1..]);

        let mut framed = Vec::new();
        framed.extend_from_slice(&(non_canonical.len() as u32).to_be_bytes());
        framed.extend_from_slice(&non_canonical);

        let mut reader = EventsReader::new(framed.as_slice());
        let err = reader.read_event().expect_err("must fail");
        assert!(matches!(err, BundleError::NonCanonicalCbor));
    }

    #[test]
    fn t_events_writer_produces_canonical_cbor() {
        let event = sample_event(0);
        let mut out = Vec::new();
        let mut writer = EventsWriter::new(&mut out);
        writer.write_event(&event).expect("write");
        writer.finish().expect("finish");

        let frame_len = u32::from_be_bytes([out[0], out[1], out[2], out[3]]) as usize;
        let payload = &out[4..4 + frame_len];
        let wire: WireEvent = ciborium::de::from_reader(payload).expect("decode");
        let mut canonical = Vec::new();
        ciborium::ser::into_writer(&wire, &mut canonical).expect("reencode");
        assert_eq!(payload, canonical.as_slice());
    }

    #[test]
    fn t_canonical_event_encoding_matches_akmon_journal_provider_call() {
        let event = Event {
            parents: vec![hash(0x10)],
            kind: EventKind::ProviderCall {
                provider_id: "anthropic".to_owned(),
                attempts: vec![AttemptRecord {
                    attempt_number: 1,
                    started_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_100)
                        .expect("ts"),
                    ended_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_101).expect("ts"),
                    status: AttemptStatus::Success,
                    request_hash: hash(0x21),
                    response_hash: Some(hash(0x22)),
                    stream_hash: Some(hash(0x23)),
                    error_message: None,
                }],
                stream_hash: Some(hash(0x24)),
            },
            emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_102).expect("ts"),
            sequence: 7,
        };

        let mut out = Vec::new();
        let mut writer = EventsWriter::with_hash_algorithm(&mut out, HashAlgorithm::Sha256);
        writer.write_event(&event).expect("write");
        writer.finish().expect("finish");

        let mut reader = EventsReader::with_hash_algorithm(out.as_slice(), HashAlgorithm::Sha256);
        let decoded = reader.read_event().expect("read").expect("some");

        let expected_hash = event.content_hash(HashAlgorithm::Sha256).expect("hash");
        let actual_hash = decoded.content_hash(HashAlgorithm::Sha256).expect("hash");
        assert_eq!(actual_hash, expected_hash);
    }

    #[test]
    fn t_canonical_event_encoding_matches_akmon_journal_tool_call() {
        let event = Event {
            parents: vec![hash(0x31)],
            kind: EventKind::ToolCall {
                tool_id: "read_file".to_owned(),
                input_hash: hash(0x32),
                output_hash: hash(0x33),
                side_effects_hash: None,
            },
            emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_200).expect("ts"),
            sequence: 9,
        };

        let mut out = Vec::new();
        let mut writer = EventsWriter::with_hash_algorithm(&mut out, HashAlgorithm::Sha256);
        writer.write_event(&event).expect("write");
        writer.finish().expect("finish");

        let mut reader = EventsReader::with_hash_algorithm(out.as_slice(), HashAlgorithm::Sha256);
        let decoded = reader.read_event().expect("read").expect("some");

        let expected_hash = event.content_hash(HashAlgorithm::Sha256).expect("hash");
        let actual_hash = decoded.content_hash(HashAlgorithm::Sha256).expect("hash");
        assert_eq!(actual_hash, expected_hash);
    }

    #[test]
    fn t_canonical_event_encoding_matches_akmon_journal_session_end() {
        let event = Event {
            parents: vec![hash(0x40)],
            kind: EventKind::SessionEnd {
                summary_hash: Some(hash(0x41)),
            },
            emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_300).expect("ts"),
            sequence: 10,
        };

        let mut out = Vec::new();
        let mut writer = EventsWriter::with_hash_algorithm(&mut out, HashAlgorithm::Sha256);
        writer.write_event(&event).expect("write");
        writer.finish().expect("finish");

        let mut reader = EventsReader::with_hash_algorithm(out.as_slice(), HashAlgorithm::Sha256);
        let decoded = reader.read_event().expect("read").expect("some");

        let expected_hash = event.content_hash(HashAlgorithm::Sha256).expect("hash");
        let actual_hash = decoded.content_hash(HashAlgorithm::Sha256).expect("hash");
        assert_eq!(actual_hash, expected_hash);
    }
}
