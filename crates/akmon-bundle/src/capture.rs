//! OTEL-import capture-level surfacing for parsed bundles (design-review F1).
//!
//! `akmon otel import` records, inside the `SessionStart` *config object*, whether
//! the source telemetry carried real message content (`capture_level = "full"`) or
//! only structural metadata (`capture_level = "structural"`). Because that config
//! object is referenced by `SessionStart`, its bytes are committed into the session
//! head — tampering with the recorded capture level changes the head and breaks both
//! integrity and signature verification.
//!
//! A bundle whose content was never captured still verifies and signature-checks
//! perfectly: integrity proves the *evidence* is intact, not that *message content*
//! was attested. A verifier that prints a bare "VERIFIED" would mislead an auditor.
//! This module lets both verifiers ([`crate::verify`] consumers) read the recorded
//! capture level out of a parsed bundle so they can surface it honestly.
//!
//! Native (non-OTEL) Akmon sessions do not carry this config object;
//! [`otel_capture_info`] returns `None` for them, which callers treat as
//! full-fidelity (a recorded native session captured its own real content).

use crate::archive::BundleContents;
use akmon_journal::EventKind;

/// OTEL-import capture metadata extracted from a parsed bundle's `SessionStart` config object.
///
/// Present only when the bundle is an `akmon otel import` (the config object carries
/// `akmon_otel_config == true`). See [`otel_capture_info`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelCaptureInfo {
    /// Recorded capture level: `"full"` (real message content present) or `"structural"`
    /// (metadata only — content was not captured by the source telemetry). `"unknown"` if the
    /// config object omits the field.
    pub capture_level: String,
    /// Pinned OpenTelemetry semantic-conventions version the importer targeted (for example
    /// `"1.37.0"`). Empty string if the config object omits the field.
    pub source_semconv: String,
}

/// Extracts OTEL-import capture info from a parsed bundle, or `None` if it is not an OTEL import.
///
/// Locates the `SessionStart` event, looks up its `config_hash` object bytes in
/// [`BundleContents::objects`], and parses them as JSON. When the object is an OTEL config
/// object (`akmon_otel_config == true`), returns its `capture_level` (default `"unknown"`) and
/// `source_semconv` (default `""`). Returns `None` for native Akmon sessions, for a missing or
/// unreadable config object, or for any non-OTEL config — callers treat `None` as full fidelity.
#[must_use]
pub fn otel_capture_info(contents: &BundleContents) -> Option<OtelCaptureInfo> {
    let config_hash = contents.events.iter().find_map(|event| match &event.kind {
        EventKind::SessionStart { config_hash, .. } => Some(config_hash),
        _ => None,
    })?;
    let bytes = contents.objects.get(config_hash)?;
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if value
        .get("akmon_otel_config")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return None;
    }
    Some(OtelCaptureInfo {
        capture_level: value
            .get("capture_level")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        source_semconv: value
            .get("source_semconv")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, Producer, SessionMetadata};
    use akmon_journal::{AGEF_SPEC_VERSION, Event, HashAlgorithm, digest_bytes};
    use std::collections::{BTreeMap, HashMap};

    fn algo() -> HashAlgorithm {
        HashAlgorithm::Sha256
    }

    fn ts(seconds: i64) -> time::OffsetDateTime {
        time::OffsetDateTime::from_unix_timestamp(seconds).expect("ts")
    }

    /// Builds a minimal two-event bundle whose `SessionStart` config object is `config_bytes`.
    fn bundle_with_config(config_bytes: Vec<u8>) -> BundleContents {
        let cwd_bytes = b"/work".to_vec();
        let cwd_hash = digest_bytes(algo(), &cwd_bytes);
        let config_hash = digest_bytes(algo(), &config_bytes);

        let start = Event {
            parents: vec![],
            kind: EventKind::SessionStart {
                cwd_hash: cwd_hash.clone(),
                config_hash: config_hash.clone(),
            },
            emitted_at: ts(1_700_000_000),
            sequence: 0,
        };
        let start_hash = start.content_hash(algo()).expect("start hash");
        let end = Event {
            parents: vec![start_hash],
            kind: EventKind::SessionEnd { summary_hash: None },
            emitted_at: ts(1_700_000_001),
            sequence: 1,
        };
        let end_hash = end.content_hash(algo()).expect("end hash");

        let objects = HashMap::from([(cwd_hash, cwd_bytes), (config_hash, config_bytes)]);
        let manifest = Manifest {
            agef_version: AGEF_SPEC_VERSION.to_owned(),
            producer: Producer {
                name: "akmon".to_owned(),
                version: "test".to_owned(),
            },
            session: SessionMetadata {
                id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                head: end_hash.to_hex(),
                created_at: "2026-05-04T14:00:00Z".to_owned(),
                ended_at: "2026-05-04T14:01:00Z".to_owned(),
            },
            hash_algorithm: "sha256".to_owned(),
            object_count: 2,
            event_count: 2,
            signatures: None,
            extra: BTreeMap::new(),
        };
        BundleContents {
            manifest,
            events: vec![start, end],
            objects,
        }
    }

    #[test]
    fn t_otel_config_structural_is_surfaced() {
        let config = serde_json::json!({
            "akmon_otel_config": true,
            "schema": "akmon-otel-config-v1",
            "capture_level": "structural",
            "source_semconv": "1.37.0",
            "provider": "openai",
            "model": "gpt-4o",
            "conversation_id": serde_json::Value::Null,
            "agent": serde_json::Value::Null,
        });
        let contents = bundle_with_config(serde_json::to_vec(&config).expect("config bytes"));
        let info = otel_capture_info(&contents).expect("otel config present");
        assert_eq!(info.capture_level, "structural");
        assert_eq!(info.source_semconv, "1.37.0");
    }

    #[test]
    fn t_otel_config_full_is_surfaced() {
        let config = serde_json::json!({
            "akmon_otel_config": true,
            "capture_level": "full",
            "source_semconv": "1.37.0",
        });
        let contents = bundle_with_config(serde_json::to_vec(&config).expect("config bytes"));
        let info = otel_capture_info(&contents).expect("otel config present");
        assert_eq!(info.capture_level, "full");
    }

    #[test]
    fn t_native_config_returns_none() {
        // A native Akmon SessionStart config object has no `akmon_otel_config` marker.
        let config = serde_json::json!({"some_native_config": true, "model": "claude"});
        let contents = bundle_with_config(serde_json::to_vec(&config).expect("config bytes"));
        assert!(otel_capture_info(&contents).is_none());
    }

    #[test]
    fn t_non_json_config_returns_none() {
        // CBOR / arbitrary bytes (native sessions encode config as CBOR) must not panic.
        let contents = bundle_with_config(vec![0xFF, 0x00, 0x42, 0x99]);
        assert!(otel_capture_info(&contents).is_none());
    }

    #[test]
    fn t_missing_capture_level_defaults_unknown() {
        let config = serde_json::json!({"akmon_otel_config": true});
        let contents = bundle_with_config(serde_json::to_vec(&config).expect("config bytes"));
        let info = otel_capture_info(&contents).expect("otel config present");
        assert_eq!(info.capture_level, "unknown");
        assert_eq!(info.source_semconv, "");
    }
}
