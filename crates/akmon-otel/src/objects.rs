//! Content-object policy: real content, metadata envelopes, not-captured
//! sentinels, and the session config object (F3/F4/F5/F9).
//!
//! Every required (non-`Option`) hash field on an AGEF event must point at some
//! object. When the trace carries opt-in content we hash that content directly;
//! otherwise we synthesize a self-describing, labeled object so the resulting
//! session is honest about what was and was not captured, and so distinct empty
//! slots never collide on the same hash.

use crate::canonical::canonical_json_bytes;
use crate::semconv::CaptureLevel;
use serde_json::{Map, Value, json};

/// Pinned OpenTelemetry semantic-conventions version this importer targets.
pub const SOURCE_SEMCONV: &str = "1.37.0";

/// Builds the canonical bytes of a metadata-envelope object (F3).
///
/// Used when real content is absent but structural metadata exists. The object
/// is self-describing: it records the slot (`field`), the capture level, the
/// source semconv version, and the sorted `gen_ai.*` metadata for that slot.
#[must_use]
pub fn metadata_envelope_bytes(field: &str, attributes: Map<String, Value>) -> Vec<u8> {
    let value = json!({
        "akmon_otel_metadata": true,
        "schema": "akmon-otel-meta-v1",
        "field": field,
        "capture_level": "structural",
        "source_semconv": SOURCE_SEMCONV,
        "attributes": Value::Object(attributes),
    });
    encode(&value)
}

/// Builds the canonical bytes of a not-captured sentinel object (F5).
///
/// Used when a required field has no data at all. The `field` discriminator
/// guarantees that different empty slots (for example `cwd` vs `tool_output`)
/// produce different object bytes, so empty slots never share a hash.
#[must_use]
pub fn not_captured_bytes(field: &str, reason: &str) -> Vec<u8> {
    let value = json!({
        "akmon_not_captured": true,
        "schema": "akmon-otel-notcaptured-v1",
        "field": field,
        "reason": reason,
    });
    encode(&value)
}

/// Builds the canonical bytes of the session config object (F9/F1).
///
/// The `capture_level` is embedded here so it is signed into the session via the
/// `SessionStart` head: tampering with the recorded capture level changes the
/// config hash and therefore the session head.
#[must_use]
pub fn config_object_bytes(
    capture_level: CaptureLevel,
    provider: Option<&str>,
    model: Option<&str>,
    conversation_id: Option<&str>,
    agent: Option<&str>,
) -> Vec<u8> {
    let value = json!({
        "akmon_otel_config": true,
        "schema": "akmon-otel-config-v1",
        "capture_level": capture_level.as_str(),
        "source_semconv": SOURCE_SEMCONV,
        "provider": opt(provider),
        "model": opt(model),
        "conversation_id": opt(conversation_id),
        "agent": opt(agent),
    });
    encode(&value)
}

/// Wraps already-parsed real content into canonical bytes (F1/F2).
///
/// `content` is the actual message/tool JSON value extracted from the trace.
#[must_use]
pub fn real_content_bytes(content: &Value) -> Vec<u8> {
    encode(content)
}

/// Converts an optional string into a JSON value (`null` when absent).
fn opt(value: Option<&str>) -> Value {
    match value {
        Some(v) => Value::String(v.to_owned()),
        None => Value::Null,
    }
}

/// Canonicalizes a value, falling back to a stable error marker that still
/// hashes deterministically if serialization ever fails (no panics).
fn encode(value: &Value) -> Vec<u8> {
    canonical_json_bytes(value)
        .unwrap_or_else(|err| format!("{{\"akmon_otel_encode_error\":{err:?}}}").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_captured_field_distinction() {
        let cwd = not_captured_bytes("cwd", "otel has no working directory");
        let tool_output = not_captured_bytes("tool_output", "tool result not captured");
        assert_ne!(cwd, tool_output);
    }

    #[test]
    fn metadata_envelope_marks_itself() {
        let mut attrs = Map::new();
        attrs.insert("model".to_owned(), json!("gpt-4o"));
        let bytes = metadata_envelope_bytes("response", attrs);
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| unreachable!());
        assert_eq!(parsed.get("akmon_otel_metadata"), Some(&json!(true)));
        assert_eq!(parsed.get("field"), Some(&json!("response")));
    }

    #[test]
    fn config_bakes_capture_level() {
        let full = config_object_bytes(
            CaptureLevel::Full,
            Some("openai"),
            Some("gpt-4o"),
            None,
            None,
        );
        let structural = config_object_bytes(
            CaptureLevel::Structural,
            Some("openai"),
            Some("gpt-4o"),
            None,
            None,
        );
        assert_ne!(full, structural);
    }
}
