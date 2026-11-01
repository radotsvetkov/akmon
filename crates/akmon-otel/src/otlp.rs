//! OTLP/JSON trace types and a typed attribute normalization layer (F7).
//!
//! These types model the subset of the OTLP `ExportTraceServiceRequest` JSON
//! shape that Akmon needs to map GenAI spans into AGEF events. They follow the
//! OTLP/JSON (protobuf JSON mapping) rules leniently but explicitly (OTEL-4):
//!
//! - 64-bit integers (`intValue`, `startTimeUnixNano`, `endTimeUnixNano`) are
//!   accepted either as JSON strings or as JSON numbers;
//! - `doubleValue` accepts JSON numbers, numeric strings, and the proto-JSON
//!   spellings `"NaN"`, `"Infinity"`, `"-Infinity"`;
//! - `bytesValue` (base64) is accepted;
//! - an empty `AnyValue` (`{}`) or a JSON `null` value is accepted as "no value".
//!
//! A malformed scalar inside one attribute (for example a non-numeric
//! `intValue`) degrades that one value to [`AnyValue::Invalid`] instead of
//! failing the whole import, so one odd attribute on an unrelated span cannot
//! reject an otherwise valid trace.

use crate::error::OtelImportError;
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Top-level OTLP/JSON `ExportTraceServiceRequest`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportTraceServiceRequest {
    #[serde(default)]
    resource_spans: Vec<ResourceSpans>,
}

/// One resource and its scoped span batches.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceSpans {
    #[serde(default)]
    scope_spans: Vec<ScopeSpans>,
    /// Producer-declared semconv schema URL for the resource.
    #[serde(default)]
    schema_url: Option<String>,
}

/// One instrumentation scope and its spans.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopeSpans {
    #[serde(default)]
    spans: Vec<Span>,
    /// Producer-declared semconv schema URL for the scope.
    #[serde(default)]
    schema_url: Option<String>,
}

/// A single OTLP span.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Span {
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    span_id: String,
    #[serde(default)]
    parent_span_id: String,
    #[serde(default)]
    start_time_unix_nano: IntLike,
    #[serde(default)]
    end_time_unix_nano: IntLike,
    #[serde(default)]
    attributes: Vec<KeyValue>,
    #[serde(default)]
    events: Vec<SpanEvent>,
    #[serde(default)]
    status: Option<Status>,
}

/// OTLP span status.
#[derive(Debug, Deserialize)]
struct Status {
    #[serde(default)]
    code: StatusCode,
    #[serde(default)]
    message: Option<String>,
}

/// A timestamped span event.
#[derive(Debug, Deserialize)]
struct SpanEvent {
    #[serde(default)]
    name: String,
    #[serde(default)]
    attributes: Vec<KeyValue>,
}

/// A single attribute key/value pair.
#[derive(Debug, Deserialize)]
struct KeyValue {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: AnyValue,
}

/// OTLP `ArrayValue` wrapper.
#[derive(Debug, Default, Deserialize)]
struct ArrayValue {
    #[serde(default)]
    values: Vec<AnyValue>,
}

/// OTLP `KvlistValue` wrapper.
#[derive(Debug, Default, Deserialize)]
struct KvList {
    #[serde(default)]
    values: Vec<KeyValue>,
}

/// A 64-bit OTLP integer transported as a JSON string or number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum IntLike {
    /// Absent or `null`.
    #[default]
    Missing,
    /// A well-formed integer.
    Valid(i128),
    /// Present but not an integer (non-numeric string, fractional number, ...).
    Invalid,
}

impl<'de> Deserialize<'de> for IntLike {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = IntLike;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an int64 as a JSON number or decimal string")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<IntLike, E> {
                Ok(IntLike::Valid(i128::from(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<IntLike, E> {
                Ok(IntLike::Valid(i128::from(v)))
            }
            fn visit_f64<E: de::Error>(self, _v: f64) -> Result<IntLike, E> {
                Ok(IntLike::Invalid)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<IntLike, E> {
                Ok(v.trim()
                    .parse::<i128>()
                    .map_or(IntLike::Invalid, IntLike::Valid))
            }
            fn visit_bool<E: de::Error>(self, _v: bool) -> Result<IntLike, E> {
                Ok(IntLike::Invalid)
            }
            fn visit_unit<E: de::Error>(self) -> Result<IntLike, E> {
                Ok(IntLike::Missing)
            }
            fn visit_none<E: de::Error>(self) -> Result<IntLike, E> {
                Ok(IntLike::Missing)
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// An OTLP status code given as an integer or as its enum name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct StatusCode(i64);

impl<'de> Deserialize<'de> for StatusCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = StatusCode;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an OTLP status code (integer or STATUS_CODE_* name)")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<StatusCode, E> {
                Ok(StatusCode(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<StatusCode, E> {
                Ok(StatusCode(i64::try_from(v).unwrap_or(-1)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<StatusCode, E> {
                Ok(StatusCode(match v {
                    "STATUS_CODE_UNSET" => 0,
                    "STATUS_CODE_OK" => 1,
                    "STATUS_CODE_ERROR" => 2,
                    other => other.parse::<i64>().unwrap_or(-1),
                }))
            }
            fn visit_unit<E: de::Error>(self) -> Result<StatusCode, E> {
                Ok(StatusCode(0))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// A `doubleValue` payload: number, numeric string, or NaN/Infinity spelling.
struct DoubleLike(Option<f64>);

impl<'de> Deserialize<'de> for DoubleLike {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = DoubleLike;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a double as a JSON number or proto-JSON string")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<DoubleLike, E> {
                Ok(DoubleLike(Some(v)))
            }
            #[allow(clippy::cast_precision_loss)]
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<DoubleLike, E> {
                Ok(DoubleLike(Some(v as f64)))
            }
            #[allow(clippy::cast_precision_loss)]
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<DoubleLike, E> {
                Ok(DoubleLike(Some(v as f64)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<DoubleLike, E> {
                Ok(DoubleLike(match v {
                    "NaN" => Some(f64::NAN),
                    "Infinity" => Some(f64::INFINITY),
                    "-Infinity" => Some(f64::NEG_INFINITY),
                    other => other.trim().parse::<f64>().ok(),
                }))
            }
            fn visit_unit<E: de::Error>(self) -> Result<DoubleLike, E> {
                Ok(DoubleLike(None))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// OTLP `AnyValue`, normalized.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) enum AnyValue {
    /// UTF-8 string value.
    String(String),
    /// Boolean value.
    Bool(bool),
    /// 64-bit integer value.
    Int(i64),
    /// Double value (may be non-finite).
    Double(f64),
    /// Base64-encoded bytes value, kept verbatim.
    Bytes(String),
    /// Array of values.
    Array(Vec<AnyValue>),
    /// Nested key/value list, in source order.
    Kvlist(Vec<(String, AnyValue)>),
    /// No value set (`{}` or `null`).
    #[default]
    Empty,
    /// A value whose scalar payload was malformed.
    Invalid,
}

impl<'de> Deserialize<'de> for AnyValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = AnyValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an OTLP AnyValue object")
            }
            fn visit_unit<E: de::Error>(self) -> Result<AnyValue, E> {
                Ok(AnyValue::Empty)
            }
            fn visit_none<E: de::Error>(self) -> Result<AnyValue, E> {
                Ok(AnyValue::Empty)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<AnyValue, A::Error> {
                let mut out = AnyValue::Empty;
                while let Some(key) = map.next_key::<String>()? {
                    let value = match key.as_str() {
                        "stringValue" => map
                            .next_value::<Option<String>>()?
                            .map_or(AnyValue::Empty, AnyValue::String),
                        "boolValue" => map
                            .next_value::<Option<bool>>()?
                            .map_or(AnyValue::Empty, AnyValue::Bool),
                        "intValue" => match map.next_value::<IntLike>()? {
                            IntLike::Missing => AnyValue::Empty,
                            IntLike::Invalid => AnyValue::Invalid,
                            IntLike::Valid(v) => {
                                i64::try_from(v).map_or(AnyValue::Invalid, AnyValue::Int)
                            }
                        },
                        "doubleValue" => match map.next_value::<DoubleLike>()? {
                            DoubleLike(Some(d)) => AnyValue::Double(d),
                            DoubleLike(None) => AnyValue::Invalid,
                        },
                        "bytesValue" => map
                            .next_value::<Option<String>>()?
                            .map_or(AnyValue::Empty, AnyValue::Bytes),
                        "arrayValue" => {
                            let arr = map.next_value::<Option<ArrayValue>>()?.unwrap_or_default();
                            AnyValue::Array(arr.values)
                        }
                        "kvlistValue" => {
                            let kv = map.next_value::<Option<KvList>>()?.unwrap_or_default();
                            AnyValue::Kvlist(
                                kv.values.into_iter().map(|e| (e.key, e.value)).collect(),
                            )
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                            continue;
                        }
                    };
                    if !matches!(value, AnyValue::Empty) {
                        out = value;
                    }
                }
                Ok(out)
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl AnyValue {
    /// Returns the string payload when this value is a `stringValue`.
    #[must_use]
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the integer payload.
    #[must_use]
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns a floating-point payload from a `doubleValue` or `intValue`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Double(d) => Some(*d),
            Self::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// Returns the array elements when this value is an `arrayValue`.
    #[must_use]
    pub(crate) fn as_array(&self) -> Option<&[AnyValue]> {
        match self {
            Self::Array(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    /// Converts this value into plain JSON (structured OTLP attributes such as an
    /// `arrayValue` of `kvlistValue` messages become JSON arrays of objects).
    ///
    /// Non-finite doubles use the proto-JSON spellings as strings, bytes become
    /// `{"bytes_base64": "..."}`, and empty/invalid values become `null`.
    #[must_use]
    pub(crate) fn to_json(&self) -> Value {
        match self {
            Self::String(s) => Value::String(s.clone()),
            Self::Bool(b) => Value::Bool(*b),
            Self::Int(v) => Value::from(*v),
            Self::Double(d) => json_double(*d),
            Self::Bytes(b) => {
                let mut obj = Map::new();
                obj.insert("bytes_base64".to_owned(), Value::String(b.clone()));
                Value::Object(obj)
            }
            Self::Array(values) => Value::Array(values.iter().map(Self::to_json).collect()),
            Self::Kvlist(entries) => {
                let mut obj = Map::new();
                for (k, v) in entries {
                    obj.insert(k.clone(), v.to_json());
                }
                Value::Object(obj)
            }
            Self::Empty | Self::Invalid => Value::Null,
        }
    }
}

/// Encodes a double as JSON, using proto-JSON strings for non-finite values.
pub(crate) fn json_double(d: f64) -> Value {
    if d.is_nan() {
        Value::String("NaN".to_owned())
    } else if d.is_infinite() {
        Value::String(if d > 0.0 { "Infinity" } else { "-Infinity" }.to_owned())
    } else {
        serde_json::Number::from_f64(d).map_or(Value::Null, Value::Number)
    }
}

/// Normalized span status (OTEL-5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SpanStatus {
    /// True when the span status code is ERROR (2).
    pub(crate) is_error: bool,
    /// The status message, when present and non-empty.
    pub(crate) message: Option<String>,
}

/// A span with its attributes flattened into a sorted, typed map (F7).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalizedSpan {
    /// Source span identifier.
    pub(crate) span_id: String,
    /// Source parent span identifier (empty for root spans).
    pub(crate) parent_span_id: String,
    /// Source trace identifier.
    pub(crate) trace_id: String,
    /// Start time in unix nanoseconds as transported.
    pub(crate) start: IntLike,
    /// End time in unix nanoseconds as transported.
    pub(crate) end: IntLike,
    /// Span status.
    pub(crate) status: SpanStatus,
    /// Flattened attribute map keyed by attribute name.
    pub(crate) attributes: BTreeMap<String, AnyValue>,
    /// Span events in OTLP source order.
    pub(crate) events: Vec<NormalizedEvent>,
}

/// A span event with its attributes flattened into a sorted, typed map.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalizedEvent {
    /// Event name.
    pub(crate) name: String,
    /// Flattened event-attribute map keyed by attribute name.
    pub(crate) attributes: BTreeMap<String, AnyValue>,
}

impl NormalizedEvent {
    /// Reads a string-typed event attribute by key.
    #[must_use]
    pub(crate) fn attr_str(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(AnyValue::as_str)
    }

    /// Reads an integer-typed event attribute by key.
    #[must_use]
    pub(crate) fn attr_i64(&self, key: &str) -> Option<i64> {
        self.attributes.get(key).and_then(AnyValue::as_i64)
    }

    /// Converts all event attributes into a JSON object (sorted by key).
    #[must_use]
    pub(crate) fn attributes_json(&self) -> Map<String, Value> {
        self.attributes
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect()
    }
}

impl NormalizedSpan {
    /// Reads a string-typed attribute by key.
    #[must_use]
    pub(crate) fn attr_str(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(AnyValue::as_str)
    }

    /// Reads an integer-typed attribute by key.
    #[must_use]
    pub(crate) fn attr_i64(&self, key: &str) -> Option<i64> {
        self.attributes.get(key).and_then(AnyValue::as_i64)
    }

    /// Reads a float-typed attribute by key.
    #[must_use]
    pub(crate) fn attr_f64(&self, key: &str) -> Option<f64> {
        self.attributes.get(key).and_then(AnyValue::as_f64)
    }

    /// Reads an array-typed attribute by key.
    #[must_use]
    pub(crate) fn attr_array(&self, key: &str) -> Option<&[AnyValue]> {
        self.attributes.get(key).and_then(AnyValue::as_array)
    }

    /// Iterates over this span's events whose name equals `name`, in source order.
    pub(crate) fn events_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a NormalizedEvent> {
        self.events.iter().filter(move |e| e.name == name)
    }
}

/// A parsed, normalized trace.
#[derive(Debug)]
pub(crate) struct ParsedTrace {
    /// Every span, flattened across resource/scope batches, in source order.
    pub(crate) spans: Vec<NormalizedSpan>,
    /// Distinct non-empty producer-declared schema URLs (resource and scope).
    pub(crate) schema_urls: BTreeSet<String>,
}

/// Parses OTLP/JSON bytes and flattens every span into a [`NormalizedSpan`].
///
/// Values are moved (not cloned) out of the deserialized request (OTEL-9).
///
/// # Errors
///
/// Returns [`OtelImportError::Parse`] when the bytes are not a valid OTLP/JSON
/// `ExportTraceServiceRequest`.
pub(crate) fn parse_and_normalize(trace_json: &[u8]) -> Result<ParsedTrace, OtelImportError> {
    let request: ExportTraceServiceRequest = serde_json::from_slice(trace_json)
        .map_err(|err| OtelImportError::Parse(err.to_string()))?;

    let mut spans = Vec::new();
    let mut schema_urls = BTreeSet::new();
    for resource_spans in request.resource_spans {
        if let Some(url) = resource_spans.schema_url.filter(|u| !u.trim().is_empty()) {
            schema_urls.insert(url);
        }
        for scope_spans in resource_spans.scope_spans {
            if let Some(url) = scope_spans.schema_url.filter(|u| !u.trim().is_empty()) {
                schema_urls.insert(url);
            }
            spans.extend(scope_spans.spans.into_iter().map(normalize_span));
        }
    }
    Ok(ParsedTrace { spans, schema_urls })
}

/// Flattens attributes into a map (last-writer-wins on malformed duplicate keys).
fn flatten(attributes: Vec<KeyValue>) -> BTreeMap<String, AnyValue> {
    attributes
        .into_iter()
        .map(|kv| (kv.key, kv.value))
        .collect()
}

/// Flattens one [`Span`] into a [`NormalizedSpan`], moving its values.
fn normalize_span(span: Span) -> NormalizedSpan {
    let status = span
        .status
        .map_or_else(SpanStatus::default, |s| SpanStatus {
            is_error: s.code.0 == 2,
            message: s.message.filter(|m| !m.trim().is_empty()),
        });
    let events = span
        .events
        .into_iter()
        .map(|event| NormalizedEvent {
            name: event.name,
            attributes: flatten(event.attributes),
        })
        .collect();
    NormalizedSpan {
        span_id: span.span_id,
        parent_span_id: span.parent_span_id,
        trace_id: span.trace_id,
        start: span.start_time_unix_nano,
        end: span.end_time_unix_nano,
        status,
        attributes: flatten(span.attributes),
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_span(span_json: &str) -> NormalizedSpan {
        let json = format!(r#"{{"resourceSpans":[{{"scopeSpans":[{{"spans":[{span_json}]}}]}}]}}"#);
        let mut parsed = parse_and_normalize(json.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
        parsed.spans.remove(0)
    }

    fn attr(span: &NormalizedSpan, key: &str) -> AnyValue {
        span.attributes
            .get(key)
            .cloned()
            .unwrap_or(AnyValue::Invalid)
    }

    #[test]
    fn int_value_accepts_string_and_number() {
        let s = one_span(
            r#"{"attributes":[{"key":"a","value":{"intValue":"512"}},{"key":"b","value":{"intValue":31}}]}"#,
        );
        assert_eq!(attr(&s, "a"), AnyValue::Int(512));
        assert_eq!(attr(&s, "b"), AnyValue::Int(31));
        assert_eq!(attr(&s, "a").as_f64(), Some(512.0));
    }

    #[test]
    fn timestamps_accept_string_and_number() {
        let s = one_span(
            r#"{"startTimeUnixNano":1748000000000000000,"endTimeUnixNano":"1748000001000000000"}"#,
        );
        assert_eq!(s.start, IntLike::Valid(1_748_000_000_000_000_000));
        assert_eq!(s.end, IntLike::Valid(1_748_000_001_000_000_000));
    }

    #[test]
    fn missing_and_invalid_timestamps_are_distinguished() {
        let s = one_span(r#"{"name":"x","endTimeUnixNano":"abc"}"#);
        assert_eq!(s.start, IntLike::Missing);
        assert_eq!(s.end, IntLike::Invalid);
    }

    #[test]
    fn bytes_empty_null_and_nonfinite_doubles_are_accepted() {
        let s = one_span(
            r#"{"attributes":[
                {"key":"bytes","value":{"bytesValue":"aGVsbG8="}},
                {"key":"empty","value":{}},
                {"key":"null","value":null},
                {"key":"nan","value":{"doubleValue":"NaN"}},
                {"key":"inf","value":{"doubleValue":"Infinity"}},
                {"key":"ninf","value":{"doubleValue":"-Infinity"}},
                {"key":"badint","value":{"intValue":"twelve"}}
            ]}"#,
        );
        assert_eq!(attr(&s, "bytes"), AnyValue::Bytes("aGVsbG8=".to_owned()));
        assert_eq!(attr(&s, "empty"), AnyValue::Empty);
        assert_eq!(attr(&s, "null"), AnyValue::Empty);
        assert!(matches!(attr(&s, "nan"), AnyValue::Double(d) if d.is_nan()));
        assert_eq!(attr(&s, "inf"), AnyValue::Double(f64::INFINITY));
        assert_eq!(attr(&s, "ninf"), AnyValue::Double(f64::NEG_INFINITY));
        assert_eq!(attr(&s, "badint"), AnyValue::Invalid);
        assert_eq!(attr(&s, "nan").to_json(), Value::String("NaN".to_owned()));
    }

    #[test]
    fn structured_any_value_converts_to_json() {
        let s = one_span(
            r#"{"attributes":[{"key":"m","value":{"arrayValue":{"values":[{"kvlistValue":{"values":[{"key":"role","value":{"stringValue":"user"}},{"key":"n","value":{"intValue":"2"}}]}}]}}}]}"#,
        );
        assert_eq!(
            attr(&s, "m").to_json(),
            serde_json::json!([{"role": "user", "n": 2}])
        );
    }

    #[test]
    fn status_code_parses_int_and_name() {
        let a = one_span(r#"{"status":{"code":2,"message":"boom"}}"#);
        assert!(a.status.is_error);
        assert_eq!(a.status.message.as_deref(), Some("boom"));
        let b = one_span(r#"{"status":{"code":"STATUS_CODE_ERROR"}}"#);
        assert!(b.status.is_error);
        let c = one_span(r#"{"status":{"code":1}}"#);
        assert!(!c.status.is_error);
    }

    #[test]
    fn schema_urls_are_collected() {
        let json = br#"{"resourceSpans":[{"schemaUrl":"https://opentelemetry.io/schemas/1.40.0","scopeSpans":[{"schemaUrl":"","spans":[]}]}]}"#;
        let parsed = parse_and_normalize(json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.schema_urls.len(), 1);
    }

    #[test]
    fn event_attributes_round_trip_into_normalized_event() {
        let s = one_span(
            r#"{"events":[{"name":"gen_ai.user.message","attributes":[{"key":"role","value":{"stringValue":"user"}},{"key":"index","value":{"intValue":"0"}}]}]}"#,
        );
        let event = s
            .events_named("gen_ai.user.message")
            .next()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(event.attr_str("role"), Some("user"));
        assert_eq!(event.attr_i64("index"), Some(0));
    }

    #[test]
    fn invalid_json_is_parse_error() {
        let err = parse_and_normalize(b"not json").expect_err("must fail");
        assert!(matches!(err, OtelImportError::Parse(_)));
    }
}
