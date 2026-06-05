//! Import OpenTelemetry GenAI traces (semconv v1.37.0 structured form) into AGEF
//! sessions (Akmon Item 9.1).
//!
//! [`import_otel_genai`] parses an OTLP/JSON `ExportTraceServiceRequest`, maps its
//! GenAI spans onto AGEF [`EventKind`](akmon_journal::EventKind) values in a total
//! deterministic order, stores every referenced content object in an
//! [`ObjectStore`](akmon_journal::ObjectStore), and appends the events to a fresh
//! [`SessionGraph`](akmon_journal::SessionGraph). The graph auto-links parents,
//! sequence numbers, and the head, so the produced session is a valid AGEF merkle
//! chain that passes `graph.verify()?.is_clean()`.
//!
//! Content is opt-in in the GenAI conventions and is frequently absent. When real
//! message/tool content is present the importer hashes it directly and reports
//! [`CaptureLevel::Full`]; when only structural metadata is present it fills the
//! required hash fields with self-describing labeled objects and reports
//! [`CaptureLevel::Structural`]. The capture level is baked into the session
//! config object, and therefore into the signed head.

#![warn(missing_docs)]

mod canonical;
mod error;
mod objects;
mod otlp;
mod semconv;

pub use canonical::canonical_json_bytes;
pub use error::OtelImportError;
pub use otlp::{AnyValue, ExportTraceServiceRequest, NormalizedSpan, Span, parse_and_normalize};
pub use semconv::{CaptureLevel, Operation, SEMCONV_VERSION};

use akmon_journal::{AttemptRecord, AttemptStatus, EventKind, Hash, ObjectStore, SessionGraph};
use objects::{
    config_object_bytes, metadata_envelope_bytes, not_captured_bytes, real_content_bytes,
};
use serde_json::{Map, Value};
use time::OffsetDateTime;
use uuid::Uuid;

/// Outcome of importing one OTLP/JSON GenAI trace into an AGEF session.
#[derive(Debug, Clone)]
pub struct ImportReport {
    /// Identifier of the session the events were appended to.
    pub session_id: Uuid,
    /// Whether real content was captured, or only structural metadata.
    pub capture_level: CaptureLevel,
    /// Number of provider-call events emitted.
    pub provider_calls: u64,
    /// Number of tool-call events emitted.
    pub tool_calls: u64,
    /// Number of user/assistant turn events emitted (with real content).
    pub turns_emitted: u64,
    /// Number of turn events suppressed because only metadata (no real content)
    /// was available for them.
    pub turns_suppressed_no_content: u64,
    /// The pinned semconv version this import targeted.
    pub semconv_version: String,
}

/// Imports an OTLP/JSON GenAI trace into a fresh, empty AGEF session.
///
/// The trace must be a single OTLP `ExportTraceServiceRequest` JSON object using
/// the semconv >= v1.37.0 structured GenAI attributes. Spans are mapped to AGEF
/// events in a total deterministic order (start time ascending, then span id
/// ascending), bracketed by exactly one synthetic
/// [`SessionStart`](akmon_journal::EventKind::SessionStart) and one terminal
/// [`SessionEnd`](akmon_journal::EventKind::SessionEnd). Content objects are
/// stored via `store.put`; events are appended via `graph.append`.
///
/// `graph` must be freshly opened and empty.
///
/// # Errors
///
/// - [`OtelImportError::Parse`] when the bytes are not valid OTLP/JSON.
/// - [`OtelImportError::EmptyTrace`] when no spans are present.
/// - [`OtelImportError::LegacySemconvUnsupported`] when the legacy (<= v1.36)
///   message-event form is detected (F8).
/// - [`OtelImportError::MultipleSessions`] when more than one
///   `gen_ai.conversation.id` is present (F6).
/// - [`OtelImportError::Journal`] when an object-store or graph operation fails.
pub fn import_otel_genai<S: ObjectStore, G: SessionGraph>(
    trace_json: &[u8],
    store: &S,
    graph: &mut G,
) -> Result<ImportReport, OtelImportError> {
    let mut spans = parse_and_normalize(trace_json)?;
    if spans.is_empty() {
        return Err(OtelImportError::EmptyTrace);
    }

    detect_legacy(&spans)?;
    resolve_session_key(&spans)?;

    // Total deterministic order: (start_unix_nano asc, span_id asc).
    spans.sort_by(|a, b| {
        a.start_unix_nano
            .cmp(&b.start_unix_nano)
            .then_with(|| a.span_id.cmp(&b.span_id))
    });

    // First pass: determine capture level and session-level config attributes so
    // the SessionStart config object (which is signed into the head) is correct.
    let capture_level = compute_capture_level(&spans);
    let session_meta = collect_session_meta(&spans);
    let session_id = graph.session_id();

    let mut emitter = Emitter::new(store, graph, capture_level);

    // SessionStart (first), with cwd sentinel + config object.
    emitter.emit_session_start(&session_meta)?;

    let mut user_turn_emitted = false;
    for span in &spans {
        let Some(op) = span_operation(span) else {
            continue;
        };
        match op {
            Operation::ProviderCall | Operation::Embeddings => {
                emitter.emit_provider_call_span(span, &mut user_turn_emitted)?;
            }
            Operation::ExecuteTool => {
                emitter.emit_tool_call_span(span)?;
            }
            // create_agent / invoke_agent / invoke_workflow / retrieval contribute
            // session structure (root resolution, config) but emit no per-span
            // AGEF event in v1.
            Operation::CreateAgent
            | Operation::InvokeAgent
            | Operation::InvokeWorkflow
            | Operation::Retrieval => {}
        }
    }

    // SessionEnd (terminal).
    emitter.emit_session_end()?;

    Ok(ImportReport {
        session_id,
        capture_level,
        provider_calls: emitter.provider_calls,
        tool_calls: emitter.tool_calls,
        turns_emitted: emitter.turns_emitted,
        turns_suppressed_no_content: emitter.turns_suppressed_no_content,
        semconv_version: SEMCONV_VERSION.to_owned(),
    })
}

/// Returns the recognized GenAI operation for a span, if any.
fn span_operation(span: &NormalizedSpan) -> Option<Operation> {
    span.attr_str(semconv::OPERATION_NAME)
        .and_then(Operation::from_name)
}

/// Rejects legacy (<= v1.36) traces (F8).
///
/// A trace is legacy when any span carries a `gen_ai.`-prefixed span event, or
/// when the deprecated `gen_ai.system` attribute appears anywhere while no
/// structured `gen_ai.input.messages` / `gen_ai.output.messages` attribute is
/// present anywhere in the trace.
fn detect_legacy(spans: &[NormalizedSpan]) -> Result<(), OtelImportError> {
    let any_legacy_event = spans.iter().any(|s| s.has_legacy_genai_event);
    if any_legacy_event {
        return Err(OtelImportError::LegacySemconvUnsupported);
    }

    let any_deprecated_system = spans.iter().any(|s| s.has_attr(semconv::SYSTEM_DEPRECATED));
    let any_structured_messages = spans
        .iter()
        .any(|s| s.has_attr(semconv::INPUT_MESSAGES) || s.has_attr(semconv::OUTPUT_MESSAGES));
    if any_deprecated_system && !any_structured_messages {
        return Err(OtelImportError::LegacySemconvUnsupported);
    }
    Ok(())
}

/// Resolves the session key and rejects multi-session traces (F6).
///
/// v1 assumes one session per trace. If more than one distinct
/// `gen_ai.conversation.id` is present, the trace is refused rather than merged.
fn resolve_session_key(spans: &[NormalizedSpan]) -> Result<String, OtelImportError> {
    let mut conversation_ids: Vec<&str> = spans
        .iter()
        .filter_map(|s| s.attr_str(semconv::CONVERSATION_ID))
        .collect();
    conversation_ids.sort_unstable();
    conversation_ids.dedup();
    if conversation_ids.len() > 1 {
        return Err(OtelImportError::MultipleSessions);
    }
    if let Some(first) = conversation_ids.first() {
        return Ok((*first).to_owned());
    }

    // Else: span id of the root invoke_agent / invoke_workflow span.
    if let Some(root) = spans.iter().find(|s| {
        s.parent_span_id.is_empty()
            && span_operation(s)
                .map(Operation::is_session_root)
                .unwrap_or(false)
    }) {
        return Ok(root.span_id.clone());
    }

    // Else: the trace id (first non-empty).
    if let Some(trace_id) = spans
        .iter()
        .map(|s| s.trace_id.as_str())
        .find(|t| !t.is_empty())
    {
        return Ok(trace_id.to_owned());
    }

    // No identifier available; fall back to an empty key (single anonymous session).
    Ok(String::new())
}

/// Whether any span carries real message/tool content (F-capture).
fn compute_capture_level(spans: &[NormalizedSpan]) -> CaptureLevel {
    let any_content = spans.iter().any(|s| {
        s.has_attr(semconv::SYSTEM_INSTRUCTIONS)
            || s.has_attr(semconv::INPUT_MESSAGES)
            || s.has_attr(semconv::OUTPUT_MESSAGES)
            || s.has_attr(semconv::TOOL_CALL_ARGUMENTS)
            || s.has_attr(semconv::TOOL_CALL_RESULT)
    });
    if any_content {
        CaptureLevel::Full
    } else {
        CaptureLevel::Structural
    }
}

/// Session-level metadata captured for the config object.
struct SessionMeta {
    provider: Option<String>,
    model: Option<String>,
    conversation_id: Option<String>,
    agent: Option<String>,
}

/// Gathers session-level config metadata across all spans.
fn collect_session_meta(spans: &[NormalizedSpan]) -> SessionMeta {
    let provider = spans
        .iter()
        .find_map(|s| s.attr_str(semconv::PROVIDER_NAME))
        .or_else(|| {
            spans
                .iter()
                .find_map(|s| s.attr_str(semconv::SYSTEM_DEPRECATED))
        })
        .map(str::to_owned)
        .or_else(|| Some("unknown".to_owned()));

    let model = spans
        .iter()
        .find_map(|s| s.attr_str(semconv::REQUEST_MODEL))
        .or_else(|| {
            spans
                .iter()
                .find_map(|s| s.attr_str(semconv::RESPONSE_MODEL))
        })
        .map(str::to_owned);

    let conversation_id = spans
        .iter()
        .find_map(|s| s.attr_str(semconv::CONVERSATION_ID))
        .map(str::to_owned);

    // Agent identity: span id of a root invoke_agent/invoke_workflow span, if any.
    let agent = spans
        .iter()
        .find(|s| {
            s.parent_span_id.is_empty()
                && span_operation(s)
                    .map(Operation::is_session_root)
                    .unwrap_or(false)
        })
        .map(|s| s.span_id.clone());

    SessionMeta {
        provider,
        model,
        conversation_id,
        agent,
    }
}

/// Drives object storage and event emission against the target store and graph.
struct Emitter<'a, S: ObjectStore, G: SessionGraph> {
    store: &'a S,
    graph: &'a mut G,
    capture_level: CaptureLevel,
    provider_calls: u64,
    tool_calls: u64,
    turns_emitted: u64,
    turns_suppressed_no_content: u64,
}

impl<'a, S: ObjectStore, G: SessionGraph> Emitter<'a, S, G> {
    fn new(store: &'a S, graph: &'a mut G, capture_level: CaptureLevel) -> Self {
        Self {
            store,
            graph,
            capture_level,
            provider_calls: 0,
            tool_calls: 0,
            turns_emitted: 0,
            turns_suppressed_no_content: 0,
        }
    }

    /// Stores object bytes and returns the resulting hash.
    fn put(&self, bytes: &[u8]) -> Result<Hash, OtelImportError> {
        Ok(self.store.put(bytes)?)
    }

    /// Emits the synthetic `SessionStart` with cwd sentinel + config object.
    fn emit_session_start(&mut self, meta: &SessionMeta) -> Result<(), OtelImportError> {
        let cwd_hash = self.put(&not_captured_bytes(
            "cwd",
            "otel traces carry no working directory",
        ))?;
        let config_hash = self.put(&config_object_bytes(
            self.capture_level,
            meta.provider.as_deref(),
            meta.model.as_deref(),
            meta.conversation_id.as_deref(),
            meta.agent.as_deref(),
        ))?;
        self.graph.append(EventKind::SessionStart {
            cwd_hash,
            config_hash,
        })?;
        Ok(())
    }

    /// Emits a `ProviderCall` for a chat/generate/completion/embeddings span,
    /// plus the user/assistant turns that have real content.
    fn emit_provider_call_span(
        &mut self,
        span: &NormalizedSpan,
        user_turn_emitted: &mut bool,
    ) -> Result<(), OtelImportError> {
        // UserTurn first, before the ProviderCall, only for real user content and
        // only once per session.
        if !*user_turn_emitted {
            if let Some(prompt) = extract_user_content(span) {
                let prompt_hash = self.put(&real_content_bytes(&prompt))?;
                self.graph.append(EventKind::UserTurn { prompt_hash })?;
                self.turns_emitted += 1;
                *user_turn_emitted = true;
            }
        }

        // ProviderCall (always emitted for a provider span).
        let request_hash = self.put(&self.request_object_bytes(span))?;
        let response_hash = self.put(&self.response_object_bytes(span))?;

        let (status, error_message) = match span.attr_str(semconv::ERROR_TYPE) {
            None => (AttemptStatus::Success, None),
            Some(error_type) => (
                AttemptStatus::Other(error_type.to_owned()),
                Some(error_type.to_owned()),
            ),
        };

        let attempt = AttemptRecord {
            attempt_number: 1,
            started_at: nanos_to_time(span.start_unix_nano),
            ended_at: nanos_to_time(span.end_unix_nano),
            status,
            request_hash,
            response_hash: Some(response_hash),
            stream_hash: None,
            error_message,
        };
        let provider_id = span
            .attr_str(semconv::PROVIDER_NAME)
            .or_else(|| span.attr_str(semconv::SYSTEM_DEPRECATED))
            .unwrap_or("unknown")
            .to_owned();
        self.graph.append(EventKind::ProviderCall {
            provider_id,
            attempts: vec![attempt],
            stream_hash: None,
        })?;
        self.provider_calls += 1;

        // AssistantTurn after the ProviderCall, only for real assistant content.
        match extract_assistant_content(span) {
            Some((message, tool_calls)) => {
                let message_hash = self.put(&real_content_bytes(&message))?;
                let tool_calls_hash = match tool_calls {
                    Some(tc) => Some(self.put(&real_content_bytes(&tc))?),
                    None => None,
                };
                self.graph.append(EventKind::AssistantTurn {
                    message_hash,
                    tool_calls_hash,
                })?;
                self.turns_emitted += 1;
            }
            None => {
                // Metadata-only: do not synthesize a turn with an envelope/sentinel.
                self.turns_suppressed_no_content += 1;
            }
        }
        Ok(())
    }

    /// Emits a `ToolCall` for an execute_tool span.
    fn emit_tool_call_span(&mut self, span: &NormalizedSpan) -> Result<(), OtelImportError> {
        let tool_name = span.attr_str(semconv::TOOL_NAME).unwrap_or("unknown");

        let input_hash = match parse_content_attr(span, semconv::TOOL_CALL_ARGUMENTS) {
            Some(args) => self.put(&real_content_bytes(&args))?,
            None => {
                let mut attrs = Map::new();
                attrs.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
                if let Some(call_id) = span.attr_str(semconv::TOOL_CALL_ID) {
                    attrs.insert("tool_call_id".to_owned(), Value::String(call_id.to_owned()));
                }
                self.put(&metadata_envelope_bytes("tool_input", attrs))?
            }
        };

        let output_hash = match parse_content_attr(span, semconv::TOOL_CALL_RESULT) {
            Some(result) => self.put(&real_content_bytes(&result))?,
            None => {
                let mut attrs = Map::new();
                attrs.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
                self.put(&metadata_envelope_bytes("tool_output", attrs))?
            }
        };

        self.graph.append(EventKind::ToolCall {
            tool_id: tool_name.to_owned(),
            input_hash,
            output_hash,
            side_effects_hash: None,
        })?;
        self.tool_calls += 1;
        Ok(())
    }

    /// Emits the terminal synthetic `SessionEnd`.
    fn emit_session_end(&mut self) -> Result<(), OtelImportError> {
        self.graph
            .append(EventKind::SessionEnd { summary_hash: None })?;
        Ok(())
    }

    /// Builds the request-slot object bytes: real input content if present, else
    /// a request metadata envelope.
    fn request_object_bytes(&self, span: &NormalizedSpan) -> Vec<u8> {
        if let Some(content) = extract_request_content(span) {
            return real_content_bytes(&content);
        }
        let mut attrs = Map::new();
        if let Some(model) = span.attr_str(semconv::REQUEST_MODEL) {
            attrs.insert("model".to_owned(), Value::String(model.to_owned()));
        }
        if let Some(temp) = span.attr_f64(semconv::REQUEST_TEMPERATURE) {
            attrs.insert("temperature".to_owned(), json_number(temp));
        }
        if let Some(max_tokens) = span.attr_i64(semconv::REQUEST_MAX_TOKENS) {
            attrs.insert("max_tokens".to_owned(), Value::from(max_tokens));
        }
        if let Some(input_tokens) = span.attr_i64(semconv::USAGE_INPUT_TOKENS) {
            attrs.insert("input_tokens".to_owned(), Value::from(input_tokens));
        }
        metadata_envelope_bytes("request", attrs)
    }

    /// Builds the response-slot object bytes: real output content if present, else
    /// a response metadata envelope.
    fn response_object_bytes(&self, span: &NormalizedSpan) -> Vec<u8> {
        if let Some(content) = parse_content_attr(span, semconv::OUTPUT_MESSAGES) {
            return real_content_bytes(&content);
        }
        let mut attrs = Map::new();
        if let Some(model) = span.attr_str(semconv::RESPONSE_MODEL) {
            attrs.insert("model".to_owned(), Value::String(model.to_owned()));
        }
        if let Some(output_tokens) = span.attr_i64(semconv::USAGE_OUTPUT_TOKENS) {
            attrs.insert("output_tokens".to_owned(), Value::from(output_tokens));
        }
        if let Some(reasons) = span.attr_array(semconv::FINISH_REASONS) {
            let reasons: Vec<Value> = reasons
                .iter()
                .filter_map(|v| v.as_str().map(|s| Value::String(s.to_owned())))
                .collect();
            attrs.insert("finish_reasons".to_owned(), Value::Array(reasons));
        }
        if let Some(response_id) = span.attr_str(semconv::RESPONSE_ID) {
            attrs.insert(
                "response_id".to_owned(),
                Value::String(response_id.to_owned()),
            );
        }
        metadata_envelope_bytes("response", attrs)
    }
}

/// Converts a unix-nanosecond instant into an [`OffsetDateTime`], clamping to the
/// epoch on out-of-range input (no panics).
fn nanos_to_time(nanos: i128) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(nanos).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Builds a JSON number from an f64, falling back to null for non-finite values.
fn json_number(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}

/// Parses a content attribute that is itself a JSON document encoded as a string.
///
/// Returns the parsed JSON value when the attribute is present; if the string is
/// not valid JSON it is wrapped as a plain string value so the real (verbatim)
/// content is still captured.
fn parse_content_attr(span: &NormalizedSpan, key: &str) -> Option<Value> {
    let raw = span.attr_str(key)?;
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => Some(value),
        Err(_) => Some(Value::String(raw.to_owned())),
    }
}

/// Builds the full request content object from real input attributes (system
/// instructions + input messages). Returns `None` when neither is present.
fn extract_request_content(span: &NormalizedSpan) -> Option<Value> {
    let system = parse_content_attr(span, semconv::SYSTEM_INSTRUCTIONS);
    let input = parse_content_attr(span, semconv::INPUT_MESSAGES);
    if system.is_none() && input.is_none() {
        return None;
    }
    let mut obj = Map::new();
    if let Some(system) = system {
        obj.insert("system_instructions".to_owned(), system);
    }
    if let Some(input) = input {
        obj.insert("input_messages".to_owned(), input);
    }
    Some(Value::Object(obj))
}

/// Extracts the first user message's content from `gen_ai.input.messages`.
///
/// Returns the user message value when real input content is present and contains
/// a `role == "user"` entry; otherwise `None`.
fn extract_user_content(span: &NormalizedSpan) -> Option<Value> {
    let input = parse_content_attr(span, semconv::INPUT_MESSAGES)?;
    let messages = input.as_array()?;
    let user = messages
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))?;
    Some(user.clone())
}

/// Extracts assistant message content from `gen_ai.output.messages`.
///
/// Returns `Some((message_value, tool_calls_value))` when real output content is
/// present and contains an assistant message: `message_value` holds the
/// assistant's text parts (an array, possibly empty), and `tool_calls_value` is
/// `Some` when the assistant message contains `tool_call` parts.
fn extract_assistant_content(span: &NormalizedSpan) -> Option<(Value, Option<Value>)> {
    let output = parse_content_attr(span, semconv::OUTPUT_MESSAGES)?;
    let messages = output.as_array()?;
    let assistant = messages
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))?;

    let parts = assistant
        .get("parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut text_parts = Vec::new();
    let mut tool_call_parts = Vec::new();
    for part in &parts {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
        if part_type == "tool_call" {
            tool_call_parts.push(part.clone());
        } else {
            text_parts.push(part.clone());
        }
    }

    let message_value = Value::Array(text_parts);
    let tool_calls_value = if tool_call_parts.is_empty() {
        None
    } else {
        Some(Value::Array(tool_call_parts))
    };
    Some((message_value, tool_calls_value))
}

/// Re-exports the canonical semconv key constants for downstream consumers.
pub mod keys {
    pub use crate::semconv::{
        CONVERSATION_ID, ERROR_TYPE, FINISH_REASONS, INPUT_MESSAGES, OPERATION_NAME,
        OUTPUT_MESSAGES, PROVIDER_NAME, REQUEST_MAX_TOKENS, REQUEST_MODEL, REQUEST_TEMPERATURE,
        RESPONSE_ID, RESPONSE_MODEL, SYSTEM_DEPRECATED, SYSTEM_INSTRUCTIONS, TOOL_CALL_ARGUMENTS,
        TOOL_CALL_ID, TOOL_CALL_RESULT, TOOL_DESCRIPTION, TOOL_NAME, TOOL_TYPE, USAGE_INPUT_TOKENS,
        USAGE_OUTPUT_TOKENS,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_journal::{Event, HashAlgorithm, MemoryObjectStore, MemorySessionGraph};
    use std::sync::Arc;

    const FIXTURE_A: &[u8] = br#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}},{"key":"gen_ai.system_instructions","value":{"stringValue":"[{\"type\":\"text\",\"content\":\"You are a helpful weather assistant.\"}]"}},{"key":"gen_ai.input.messages","value":{"stringValue":"[{\"role\":\"user\",\"parts\":[{\"type\":\"text\",\"content\":\"Weather in Paris?\"}]}]"}},{"key":"gen_ai.output.messages","value":{"stringValue":"[{\"role\":\"assistant\",\"parts\":[{\"type\":\"tool_call\",\"id\":\"call_x\",\"name\":\"get_weather\",\"arguments\":{\"location\":\"Paris\"}}],\"finish_reason\":\"tool_calls\"}]"}}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}},{"key":"gen_ai.tool.call.arguments","value":{"stringValue":"{\"location\":\"Paris\"}"}},{"key":"gen_ai.tool.call.result","value":{"stringValue":"rainy, 57F"}}]}]}]}]}"#;

    const FIXTURE_B: &[u8] = br#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}}]}]}]}]}"#;

    const FIXTURE_LEGACY: &[u8] = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"abcd","spanId":"1111","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}}],"events":[{"name":"gen_ai.user.message"}]}]}]}]}"#;

    fn stores() -> (Arc<MemoryObjectStore>, MemorySessionGraph) {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = MemorySessionGraph::open_new(Arc::clone(&store), Uuid::new_v4());
        (store, graph)
    }

    fn kinds(history: &[(Hash, Event)]) -> Vec<&EventKind> {
        history.iter().map(|(_, e)| &e.kind).collect()
    }

    #[test]
    fn fixture_a_full_capture_emits_all_turns_and_verifies() {
        let (store, mut graph) = stores();
        let report =
            import_otel_genai(FIXTURE_A, store.as_ref(), &mut graph).unwrap_or_else(|_| panic!());

        assert_eq!(report.capture_level, CaptureLevel::Full);
        assert_eq!(report.provider_calls, 1);
        assert_eq!(report.tool_calls, 1);
        assert_eq!(report.semconv_version, "1.37.0");

        let history = graph.history().unwrap_or_else(|_| unreachable!());
        let kinds = kinds(&history);

        // Exactly one SessionStart (first) and one terminal SessionEnd (last).
        assert!(matches!(
            kinds.first(),
            Some(EventKind::SessionStart { .. })
        ));
        assert!(matches!(kinds.last(), Some(EventKind::SessionEnd { .. })));
        let start_count = kinds
            .iter()
            .filter(|k| matches!(k, EventKind::SessionStart { .. }))
            .count();
        let end_count = kinds
            .iter()
            .filter(|k| matches!(k, EventKind::SessionEnd { .. }))
            .count();
        assert_eq!(start_count, 1);
        assert_eq!(end_count, 1);

        // A UserTurn, a ProviderCall, an AssistantTurn (tool_calls_hash Some), a ToolCall.
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::UserTurn { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::ProviderCall { .. }))
        );
        let assistant_with_tools = kinds.iter().any(|k| {
            matches!(
                k,
                EventKind::AssistantTurn {
                    tool_calls_hash: Some(_),
                    ..
                }
            )
        });
        assert!(
            assistant_with_tools,
            "assistant turn must carry tool_calls_hash"
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::ToolCall { .. }))
        );

        // Order: SessionStart, UserTurn, ProviderCall, AssistantTurn, ToolCall, SessionEnd.
        let user_pos = kinds
            .iter()
            .position(|k| matches!(k, EventKind::UserTurn { .. }))
            .unwrap_or_else(|| unreachable!());
        let provider_pos = kinds
            .iter()
            .position(|k| matches!(k, EventKind::ProviderCall { .. }))
            .unwrap_or_else(|| unreachable!());
        assert!(
            user_pos < provider_pos,
            "UserTurn must precede ProviderCall"
        );

        let report_verify = graph.verify().unwrap_or_else(|_| unreachable!());
        assert!(
            report_verify.is_clean(),
            "session must verify clean: {report_verify:?}"
        );
    }

    #[test]
    fn fixture_b_structural_only_suppresses_turns_and_verifies() {
        let (store, mut graph) = stores();
        let report =
            import_otel_genai(FIXTURE_B, store.as_ref(), &mut graph).unwrap_or_else(|_| panic!());

        assert_eq!(report.capture_level, CaptureLevel::Structural);
        assert_eq!(report.provider_calls, 1);
        assert_eq!(report.tool_calls, 1);
        assert!(report.turns_suppressed_no_content > 0);
        assert_eq!(report.turns_emitted, 0);

        let history = graph.history().unwrap_or_else(|_| unreachable!());
        let kinds = kinds(&history);

        // No turns.
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, EventKind::UserTurn { .. }))
        );
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, EventKind::AssistantTurn { .. }))
        );
        // ProviderCall + ToolCall present.
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::ProviderCall { .. }))
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, EventKind::ToolCall { .. }))
        );

        let report_verify = graph.verify().unwrap_or_else(|_| unreachable!());
        assert!(
            report_verify.is_clean(),
            "session must verify clean: {report_verify:?}"
        );

        // The request/response/tool objects are metadata envelopes.
        for (_, event) in &history {
            match &event.kind {
                EventKind::ProviderCall { attempts, .. } => {
                    let attempt = attempts.first().unwrap_or_else(|| unreachable!());
                    assert_metadata_envelope(store.as_ref(), &attempt.request_hash, "request");
                    let response = attempt
                        .response_hash
                        .as_ref()
                        .unwrap_or_else(|| unreachable!());
                    assert_metadata_envelope(store.as_ref(), response, "response");
                }
                EventKind::ToolCall {
                    input_hash,
                    output_hash,
                    ..
                } => {
                    assert_metadata_envelope(store.as_ref(), input_hash, "tool_input");
                    assert_metadata_envelope(store.as_ref(), output_hash, "tool_output");
                }
                _ => {}
            }
        }
    }

    fn assert_metadata_envelope(store: &MemoryObjectStore, hash: &Hash, field: &str) {
        let bytes = store
            .get(hash)
            .unwrap_or_else(|_| unreachable!())
            .unwrap_or_else(|| unreachable!());
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| unreachable!());
        assert_eq!(
            value.get("akmon_otel_metadata"),
            Some(&Value::Bool(true)),
            "object for {field} must be a metadata envelope"
        );
        assert_eq!(value.get("field").and_then(Value::as_str), Some(field));
    }

    #[test]
    fn legacy_event_form_is_rejected() {
        let (store, mut graph) = stores();
        let err = import_otel_genai(FIXTURE_LEGACY, store.as_ref(), &mut graph)
            .expect_err("legacy form must be rejected");
        assert!(matches!(err, OtelImportError::LegacySemconvUnsupported));
    }

    #[test]
    fn deprecated_system_without_messages_is_rejected() {
        let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"a","spanId":"1","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.system","value":{"stringValue":"openai"}}]}]}]}]}"#;
        let (store, mut graph) = stores();
        let err = import_otel_genai(json, store.as_ref(), &mut graph)
            .expect_err("deprecated system without messages must be rejected");
        assert!(matches!(err, OtelImportError::LegacySemconvUnsupported));
    }

    #[test]
    fn multiple_conversation_ids_rejected() {
        let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"a","spanId":"1","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.input.messages","value":{"stringValue":"[]"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-a"}}]},{"traceId":"a","spanId":"2","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"3","endTimeUnixNano":"4","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-b"}}]}]}]}]}"#;
        let (store, mut graph) = stores();
        let err = import_otel_genai(json, store.as_ref(), &mut graph)
            .expect_err("multiple conversation ids must be rejected");
        assert!(matches!(err, OtelImportError::MultipleSessions));
    }

    #[test]
    fn empty_trace_rejected() {
        let json = br#"{"resourceSpans":[]}"#;
        let (store, mut graph) = stores();
        let err = import_otel_genai(json, store.as_ref(), &mut graph)
            .expect_err("empty trace must be rejected");
        assert!(matches!(err, OtelImportError::EmptyTrace));
    }

    #[test]
    fn canonical_json_key_order_independent() {
        let a = serde_json::json!({"b": 1, "a": {"y": 2, "x": 1}});
        let b = serde_json::json!({"a": {"x": 1, "y": 2}, "b": 1});
        assert_eq!(
            canonical_json_bytes(&a).unwrap_or_else(|_| unreachable!()),
            canonical_json_bytes(&b).unwrap_or_else(|_| unreachable!())
        );
    }

    #[test]
    fn not_captured_sentinel_field_distinction() {
        // Reachable via the public surface: cwd uses the not-captured sentinel,
        // and distinct fields must hash to distinct bytes.
        let cwd = crate::objects::not_captured_bytes("cwd", "r");
        let tool_output = crate::objects::not_captured_bytes("tool_output", "r");
        assert_ne!(cwd, tool_output);
    }

    #[test]
    fn fixture_a_real_content_objects_are_not_envelopes() {
        let (store, mut graph) = stores();
        import_otel_genai(FIXTURE_A, store.as_ref(), &mut graph).unwrap_or_else(|_| panic!());
        let history = graph.history().unwrap_or_else(|_| unreachable!());
        for (_, event) in &history {
            if let EventKind::ToolCall {
                input_hash,
                output_hash,
                ..
            } = &event.kind
            {
                for hash in [input_hash, output_hash] {
                    let bytes = store
                        .get(hash)
                        .unwrap_or_else(|_| unreachable!())
                        .unwrap_or_else(|| unreachable!());
                    let value: Value =
                        serde_json::from_slice(&bytes).unwrap_or_else(|_| unreachable!());
                    assert!(
                        value.get("akmon_otel_metadata").is_none(),
                        "tool object should hold real content, not an envelope"
                    );
                }
            }
        }
    }
}
