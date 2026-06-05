//! OTLP/JSON trace types and a typed attribute normalization layer (F7).
//!
//! These types model the subset of the OTLP `ExportTraceServiceRequest` JSON
//! shape that Akmon needs to map GenAI spans into AGEF events. Per the OTLP/JSON
//! encoding, 64-bit integers (timestamps, `intValue`) are transported as JSON
//! strings; the helpers in this module parse them back into typed values.

use crate::error::OtelImportError;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Top-level OTLP/JSON `ExportTraceServiceRequest`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTraceServiceRequest {
    /// Resource-grouped span batches.
    #[serde(default)]
    pub resource_spans: Vec<ResourceSpans>,
}

/// One resource and its scoped span batches.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpans {
    /// Resource-level attributes (service.name, etc.).
    #[serde(default)]
    pub resource: Resource,
    /// Scope-grouped span batches.
    #[serde(default)]
    pub scope_spans: Vec<ScopeSpans>,
}

/// Resource-level attribute container.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// Resource attributes.
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

/// One instrumentation scope and its spans.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSpans {
    /// Spans emitted under this scope.
    #[serde(default)]
    pub spans: Vec<Span>,
}

/// A single OTLP span.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// Hex-encoded trace identifier.
    #[serde(default)]
    pub trace_id: String,
    /// Hex-encoded span identifier.
    #[serde(default)]
    pub span_id: String,
    /// Hex-encoded parent span identifier; empty/absent for root spans.
    #[serde(default)]
    pub parent_span_id: String,
    /// Span name.
    #[serde(default)]
    pub name: String,
    /// Span kind (OTLP enum as i32); defaults to 0 (unspecified).
    #[serde(default)]
    pub kind: i32,
    /// Start time in unix nanoseconds, transported as a JSON string.
    #[serde(default)]
    pub start_time_unix_nano: String,
    /// End time in unix nanoseconds, transported as a JSON string.
    #[serde(default)]
    pub end_time_unix_nano: String,
    /// Span attributes.
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
    /// Span events (used for legacy-form detection).
    #[serde(default)]
    pub events: Vec<SpanEvent>,
}

/// A timestamped span event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanEvent {
    /// Event name (legacy GenAI message events start with `gen_ai.`).
    #[serde(default)]
    pub name: String,
    /// Event attributes. For legacy (<= v1.36) GenAI message events these carry
    /// the message body (`role`, `content`, `id`, `tool_calls`, `index`,
    /// `finish_reason`, `message`).
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

/// A single attribute key/value pair.
#[derive(Debug, Clone, Deserialize)]
pub struct KeyValue {
    /// Attribute key.
    pub key: String,
    /// Attribute value.
    pub value: AnyValue,
}

/// OTLP `AnyValue` tagged union.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnyValue {
    /// UTF-8 string value.
    StringValue(String),
    /// 64-bit integer value, transported as a JSON string.
    IntValue(String),
    /// Double-precision floating-point value.
    DoubleValue(f64),
    /// Boolean value.
    BoolValue(bool),
    /// Homogeneous array of values.
    ArrayValue(ArrayValue),
    /// Nested key/value list.
    KvlistValue(KvList),
}

/// OTLP `ArrayValue` wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct ArrayValue {
    /// Array elements.
    #[serde(default)]
    pub values: Vec<AnyValue>,
}

/// OTLP `KvlistValue` wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct KvList {
    /// List entries.
    #[serde(default)]
    pub values: Vec<KeyValue>,
}

impl AnyValue {
    /// Returns the string payload when this value is a `stringValue`.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::StringValue(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the integer payload, parsing the OTLP int-string when present.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::IntValue(s) => s.parse::<i64>().ok(),
            _ => None,
        }
    }

    /// Returns the floating-point payload, accepting either a `doubleValue` or a
    /// numeric `intValue` string.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::DoubleValue(d) => Some(*d),
            Self::IntValue(s) => s.parse::<f64>().ok(),
            _ => None,
        }
    }

    /// Returns the array elements when this value is an `arrayValue`.
    #[must_use]
    pub fn as_array(&self) -> Option<&[AnyValue]> {
        match self {
            Self::ArrayValue(arr) => Some(arr.values.as_slice()),
            _ => None,
        }
    }
}

/// A span with its attributes flattened into a sorted, typed map (F7).
#[derive(Debug, Clone)]
pub struct NormalizedSpan {
    /// Source span identifier.
    pub span_id: String,
    /// Source parent span identifier (empty for root spans).
    pub parent_span_id: String,
    /// Source trace identifier.
    pub trace_id: String,
    /// Start time in unix nanoseconds (parsed; defaults to 0 when absent/invalid).
    pub start_unix_nano: i128,
    /// End time in unix nanoseconds (parsed; defaults to `start_unix_nano`).
    pub end_unix_nano: i128,
    /// Whether any span event name begins with `gen_ai.` (legacy-form signal, F8).
    pub has_legacy_genai_event: bool,
    /// Flattened attribute map keyed by attribute name.
    pub attributes: BTreeMap<String, AnyValue>,
    /// Span events, each with its own flattened attribute map. Legacy (<= v1.36)
    /// GenAI message bodies are carried here in OTLP source order.
    pub events: Vec<NormalizedEvent>,
}

/// A span event with its attributes flattened into a sorted, typed map.
///
/// Legacy (<= v1.36) GenAI message events (`gen_ai.user.message`,
/// `gen_ai.system.message`, `gen_ai.assistant.message`, `gen_ai.tool.message`,
/// `gen_ai.choice`) carry the message body as event attributes; this type exposes
/// them with the same typed accessors as [`NormalizedSpan`].
#[derive(Debug, Clone)]
pub struct NormalizedEvent {
    /// Event name.
    pub name: String,
    /// Flattened event-attribute map keyed by attribute name.
    pub attributes: BTreeMap<String, AnyValue>,
}

impl NormalizedEvent {
    /// Reads a string-typed event attribute by key.
    #[must_use]
    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(AnyValue::as_str)
    }

    /// Reads an integer-typed event attribute by key (parsing the OTLP int-string).
    #[must_use]
    pub fn attr_i64(&self, key: &str) -> Option<i64> {
        self.attributes.get(key).and_then(AnyValue::as_i64)
    }

    /// True when this event carries the given attribute key.
    #[must_use]
    pub fn has_attr(&self, key: &str) -> bool {
        self.attributes.contains_key(key)
    }
}

impl NormalizedSpan {
    /// Reads a string-typed attribute by key.
    #[must_use]
    pub fn attr_str(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(AnyValue::as_str)
    }

    /// Reads an integer-typed attribute by key (parsing the OTLP int-string).
    #[must_use]
    pub fn attr_i64(&self, key: &str) -> Option<i64> {
        self.attributes.get(key).and_then(AnyValue::as_i64)
    }

    /// Reads a float-typed attribute by key.
    #[must_use]
    pub fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attributes.get(key).and_then(AnyValue::as_f64)
    }

    /// Reads an array-typed attribute by key.
    #[must_use]
    pub fn attr_array(&self, key: &str) -> Option<&[AnyValue]> {
        self.attributes.get(key).and_then(AnyValue::as_array)
    }

    /// True when this span carries the given attribute key.
    #[must_use]
    pub fn has_attr(&self, key: &str) -> bool {
        self.attributes.contains_key(key)
    }

    /// Iterates over this span's events whose name equals `name`, in OTLP source
    /// order.
    pub fn events_named<'a>(
        &'a self,
        name: &'static str,
    ) -> impl Iterator<Item = &'a NormalizedEvent> {
        self.events.iter().filter(move |e| e.name == name)
    }
}

/// Parses OTLP/JSON bytes and flattens every span into a [`NormalizedSpan`].
///
/// # Errors
///
/// Returns [`OtelImportError::Parse`] when the bytes are not a valid OTLP/JSON
/// `ExportTraceServiceRequest`.
pub fn parse_and_normalize(trace_json: &[u8]) -> Result<Vec<NormalizedSpan>, OtelImportError> {
    let request: ExportTraceServiceRequest = serde_json::from_slice(trace_json)
        .map_err(|err| OtelImportError::Parse(err.to_string()))?;

    let mut normalized = Vec::new();
    for resource_spans in &request.resource_spans {
        for scope_spans in &resource_spans.scope_spans {
            for span in &scope_spans.spans {
                normalized.push(normalize_span(span));
            }
        }
    }
    Ok(normalized)
}

/// Parses a unix-nanosecond string; returns `None` for empty or non-numeric input.
fn parse_unix_nano(raw: &str) -> Option<i128> {
    if raw.is_empty() {
        return None;
    }
    raw.parse::<i128>().ok()
}

/// Flattens one [`Span`] into a [`NormalizedSpan`].
fn normalize_span(span: &Span) -> NormalizedSpan {
    let mut attributes = BTreeMap::new();
    for kv in &span.attributes {
        // Last-writer-wins on duplicate keys; OTLP attribute sets are unique by
        // contract, so this only matters for malformed input.
        attributes.insert(kv.key.clone(), kv.value.clone());
    }

    let start = parse_unix_nano(&span.start_time_unix_nano).unwrap_or(0);
    let end = parse_unix_nano(&span.end_time_unix_nano).unwrap_or(start);

    let has_legacy_genai_event = span
        .events
        .iter()
        .any(|event| event.name.starts_with("gen_ai."));

    // Flatten each event's attributes, preserving OTLP source order between events
    // (intra-span event order is load-bearing for multi-message legacy bodies).
    let events = span
        .events
        .iter()
        .map(|event| {
            let mut event_attributes = BTreeMap::new();
            for kv in &event.attributes {
                // Last-writer-wins on duplicate keys, matching span-attr handling.
                event_attributes.insert(kv.key.clone(), kv.value.clone());
            }
            NormalizedEvent {
                name: event.name.clone(),
                attributes: event_attributes,
            }
        })
        .collect();

    NormalizedSpan {
        span_id: span.span_id.clone(),
        parent_span_id: span.parent_span_id.clone(),
        trace_id: span.trace_id.clone(),
        start_unix_nano: start,
        end_unix_nano: end,
        has_legacy_genai_event,
        attributes,
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_string_parses_to_i64_and_f64() {
        let v = AnyValue::IntValue("512".to_owned());
        assert_eq!(v.as_i64(), Some(512));
        assert_eq!(v.as_f64(), Some(512.0));
        assert_eq!(v.as_str(), None);
    }

    #[test]
    fn double_value_parses_to_f64_only() {
        let v = AnyValue::DoubleValue(0.2);
        assert_eq!(v.as_f64(), Some(0.2));
        assert_eq!(v.as_i64(), None);
    }

    #[test]
    fn missing_fields_are_robust() {
        let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"name":"x"}]}]}]}"#;
        let spans = parse_and_normalize(json).unwrap_or_else(|_| unreachable!());
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.start_unix_nano, 0);
        assert_eq!(span.end_unix_nano, 0);
        assert!(span.span_id.is_empty());
        assert!(!span.has_legacy_genai_event);
    }

    #[test]
    fn legacy_event_detected() {
        let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"name":"x","events":[{"name":"gen_ai.user.message"}]}]}]}]}"#;
        let spans = parse_and_normalize(json).unwrap_or_else(|_| unreachable!());
        assert!(spans[0].has_legacy_genai_event);
    }

    #[test]
    fn event_attributes_round_trip_into_normalized_event() {
        let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"name":"x","events":[{"name":"gen_ai.user.message","attributes":[{"key":"role","value":{"stringValue":"user"}},{"key":"content","value":{"stringValue":"hi"}},{"key":"index","value":{"intValue":"0"}}]}]}]}]}]}"#;
        let spans = parse_and_normalize(json).unwrap_or_else(|_| unreachable!());
        let span = &spans[0];
        assert_eq!(span.events.len(), 1);
        let event = span
            .events_named("gen_ai.user.message")
            .next()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(event.attr_str("role"), Some("user"));
        assert_eq!(event.attr_str("content"), Some("hi"));
        assert_eq!(event.attr_i64("index"), Some(0));
        assert!(event.has_attr("content"));
        assert!(!event.has_attr("missing"));
    }

    #[test]
    fn invalid_json_is_parse_error() {
        let err = parse_and_normalize(b"not json").expect_err("must fail");
        assert!(matches!(err, OtelImportError::Parse(_)));
    }
}
