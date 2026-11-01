//! Content-object policy: real content, metadata envelopes, not-captured
//! sentinels, and the session config object (F3/F4/F5/F9).
//!
//! Every required (non-`Option`) hash field on an AGEF event must point at some
//! object. When the trace carries opt-in content we hash that content directly;
//! otherwise we synthesize a self-describing, labeled object so the resulting
//! session is honest about what was and was not captured, and so distinct empty
//! slots never collide on the same hash.

use crate::canonical::canonical_json_bytes;
use crate::semconv::{CaptureLevel, SEMCONV_VERSION};
use serde_json::{Map, Value, json};

/// Builds the canonical bytes of a metadata-envelope object (F3).
///
/// Used when real content is absent but structural metadata exists. The object
/// is self-describing: it records the slot (`field`), the capture level, the
/// importer's supported semconv version, and the sorted metadata for that slot.
#[must_use]
pub fn metadata_envelope_bytes(field: &str, attributes: Map<String, Value>) -> Vec<u8> {
    let value = json!({
        "akmon_otel_metadata": true,
        "schema": "akmon-otel-meta-v1",
        "field": field,
        "capture_level": "structural",
        "source_semconv": SEMCONV_VERSION,
        "attributes": Value::Object(attributes),
    });
    encode(&value)
}

/// Builds the canonical bytes of a failed tool call's output object (OTEL-5).
///
/// `ToolCall` events carry no status field, so a failed tool execution records
/// its failure in the output object: the error details plus the captured result
/// (when the source telemetry captured one).
#[must_use]
pub fn tool_error_bytes(error: Map<String, Value>, result: Option<Value>) -> Vec<u8> {
    let capture_level = if result.is_some() {
        "full"
    } else {
        "structural"
    };
    let value = json!({
        "akmon_otel_tool_error": true,
        "schema": "akmon-otel-tool-error-v1",
        "field": "tool_output",
        "capture_level": capture_level,
        "error": Value::Object(error),
        "result": result.unwrap_or(Value::Null),
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

/// Session-level facts recorded in the config object.
#[derive(Debug, Default)]
pub struct ConfigFacts {
    /// Provider identity.
    pub provider: Option<String>,
    /// Request (or response) model.
    pub model: Option<String>,
    /// `gen_ai.conversation.id`.
    pub conversation_id: Option<String>,
    /// Span id of the root `invoke_agent` / `invoke_workflow` span.
    pub agent: Option<String>,
    /// `gen_ai.agent.name` / `.id` / `.version`, when present.
    pub agent_identity: Map<String, Value>,
    /// The (single) source trace id.
    pub trace_id: Option<String>,
    /// The producer-declared semconv version(s), or `"undeclared"`.
    pub source_semconv: String,
    /// Which content forms were found (`structured_attributes`,
    /// `inference_details_event`, `legacy_events`), or `"none"`.
    pub source_form: String,
    /// Non-legacy `gen_ai.*` events recorded as structural metadata.
    pub structural_events: Vec<Value>,
}

/// Builds the canonical bytes of the session config object (F9/F1).
///
/// The `capture_level` is embedded here so it is signed into the session via the
/// `SessionStart` head: tampering with the recorded capture level changes the
/// config hash and therefore the session head. `source_semconv` records what the
/// producer *declared* (its `schemaUrl` version), never a hardcoded constant;
/// `importer_semconv` records the version this importer supports.
#[must_use]
pub fn config_object_bytes(capture_level: CaptureLevel, facts: &ConfigFacts) -> Vec<u8> {
    let mut value = json!({
        "akmon_otel_config": true,
        "schema": "akmon-otel-config-v1",
        "capture_level": capture_level.as_str(),
        "source_semconv": facts.source_semconv,
        "source_form": facts.source_form,
        "importer_semconv": SEMCONV_VERSION,
        "provider": opt(facts.provider.as_deref()),
        "model": opt(facts.model.as_deref()),
        "conversation_id": opt(facts.conversation_id.as_deref()),
        "agent": opt(facts.agent.as_deref()),
        "trace_id": opt(facts.trace_id.as_deref()),
    });
    if let Value::Object(obj) = &mut value {
        if !facts.agent_identity.is_empty() {
            obj.insert(
                "agent_identity".to_owned(),
                Value::Object(facts.agent_identity.clone()),
            );
        }
        if !facts.structural_events.is_empty() {
            obj.insert(
                "structural_events".to_owned(),
                Value::Array(facts.structural_events.clone()),
            );
        }
    }
    encode(&value)
}

/// Wraps already-parsed real content into canonical bytes (F1/F2).
#[must_use]
pub fn real_content_bytes(content: &Value) -> Vec<u8> {
    encode(content)
}

/// Converts an optional string into a JSON value (`null` when absent).
fn opt(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |v| Value::String(v.to_owned()))
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
    fn config_bakes_capture_level_and_honest_semconv() {
        let facts = ConfigFacts {
            source_semconv: "undeclared".to_owned(),
            ..ConfigFacts::default()
        };
        let full = config_object_bytes(CaptureLevel::Full, &facts);
        let structural = config_object_bytes(CaptureLevel::Structural, &facts);
        assert_ne!(full, structural);
        let parsed: Value = serde_json::from_slice(&full).unwrap_or_else(|_| unreachable!());
        assert_eq!(parsed.get("source_semconv"), Some(&json!("undeclared")));
        assert_eq!(
            parsed.get("importer_semconv"),
            Some(&json!(SEMCONV_VERSION))
        );
        assert!(parsed.get("structural_events").is_none());
    }
}
