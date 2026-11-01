//! Content extraction: structured attributes, the inference-details event, and
//! the legacy (<= v1.36) message-event reduction.
//!
//! Per span, each content slot (system instructions, input messages, output
//! messages) is resolved exactly once (OTEL-9) with this precedence:
//!
//! 1. the structured span attribute (`gen_ai.system_instructions`,
//!    `gen_ai.input.messages`, `gen_ai.output.messages`);
//! 2. the same attribute on a `gen_ai.client.inference.operation.details` event;
//! 3. the value synthesized from the legacy message events.
//!
//! All three routes produce the same canonical role+parts JSON for the same
//! logical content, so their content-object hashes match.

use crate::otlp::{AnyValue, NormalizedEvent, NormalizedSpan};
use crate::semconv;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Where a span's content came from (recorded in the config object).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ContentSources {
    /// A structured span attribute supplied at least one slot.
    pub(crate) attributes: bool,
    /// An inference-details event supplied at least one slot.
    pub(crate) details_event: bool,
    /// Legacy message events supplied at least one slot.
    pub(crate) legacy_events: bool,
}

/// The resolved content slots of one provider span.
#[derive(Debug, Default)]
pub(crate) struct SpanContent {
    pub(crate) system_instructions: Option<Value>,
    pub(crate) input_messages: Option<Value>,
    pub(crate) output_messages: Option<Value>,
    pub(crate) sources: ContentSources,
}

impl SpanContent {
    /// The request content object (system instructions + input messages), or
    /// `None` when neither slot carries real content.
    pub(crate) fn request(&self) -> Option<Value> {
        if self.system_instructions.is_none() && self.input_messages.is_none() {
            return None;
        }
        let mut obj = Map::new();
        if let Some(system) = &self.system_instructions {
            obj.insert("system_instructions".to_owned(), system.clone());
        }
        if let Some(input) = &self.input_messages {
            obj.insert("input_messages".to_owned(), input.clone());
        }
        Some(Value::Object(obj))
    }

    /// The input messages as a slice (empty when absent or not an array).
    pub(crate) fn input_slice(&self) -> &[Value] {
        self.input_messages
            .as_ref()
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }
}

/// Converts a content attribute value into JSON, treating "no content" as `None`.
///
/// String values are parsed as JSON documents (the common OTLP encoding); a
/// string that is not valid JSON is kept verbatim. Structured `arrayValue` /
/// `kvlistValue` values (preferred by the spec) are converted directly. Empty or
/// whitespace-only strings, `null`, `[]`, `{}` and `""` are absent (OTEL-1).
pub(crate) fn content_value(value: Option<&AnyValue>) -> Option<Value> {
    let json = match value? {
        AnyValue::String(raw) => {
            if raw.trim().is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.clone()))
        }
        AnyValue::Empty | AnyValue::Invalid => return None,
        other => other.to_json(),
    };
    if is_empty_json(&json) {
        None
    } else {
        Some(json)
    }
}

/// True for JSON values that carry no content.
fn is_empty_json(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// Resolves the content slots of a span once (see the module docs).
pub(crate) fn span_content(span: &NormalizedSpan) -> SpanContent {
    let details: Option<&BTreeMap<String, AnyValue>> = span
        .events_named(semconv::EVENT_INFERENCE_DETAILS)
        .next()
        .map(|e| &e.attributes);
    let mut sources = ContentSources::default();
    let mut resolve = |key: &str| -> Option<Value> {
        if let Some(v) = content_value(span.attributes.get(key)) {
            sources.attributes = true;
            return Some(v);
        }
        if let Some(v) = content_value(details.and_then(|d| d.get(key))) {
            sources.details_event = true;
            return Some(v);
        }
        None
    };
    let mut content = SpanContent {
        system_instructions: resolve(semconv::SYSTEM_INSTRUCTIONS),
        input_messages: resolve(semconv::INPUT_MESSAGES),
        output_messages: resolve(semconv::OUTPUT_MESSAGES),
        sources,
    };

    let needs_legacy = content.system_instructions.is_none()
        || content.input_messages.is_none()
        || content.output_messages.is_none();
    if needs_legacy
        && span
            .events
            .iter()
            .any(|e| semconv::SUPPORTED_LEGACY_EVENTS.contains(&e.name.as_str()))
    {
        let synth = synthesize_structured_from_legacy(span);
        let mut fill = |slot: &mut Option<Value>, value: Option<Value>| {
            if slot.is_none() && value.is_some() {
                *slot = value;
                content.sources.legacy_events = true;
            }
        };
        fill(&mut content.system_instructions, synth.system_instructions);
        fill(&mut content.input_messages, synth.input_messages);
        fill(&mut content.output_messages, synth.output_messages);
    }
    content
}

/// One assistant turn extracted from the output messages (OTEL-12).
#[derive(Debug)]
pub(crate) struct AssistantContent {
    /// Text parts only (an array, possibly empty).
    pub(crate) message: Value,
    /// `tool_call` parts, when present.
    pub(crate) tool_calls: Option<Value>,
    /// Number of non-text, non-tool-call parts (reasoning, uri, blob, file, ...)
    /// that are kept only in the provider-call response object, not in the turn.
    pub(crate) parts_not_in_turn: u64,
}

/// Extracts one [`AssistantContent`] per assistant message (choice) in the output.
///
/// Only `text` parts (and untyped parts) become the turn's message; `tool_call`
/// parts become its tool calls; any other part type (v1.38 `reasoning`, `uri`,
/// `blob`, `file`, ...) is not hashed as assistant text. Those parts remain in the
/// provider call's response object, which holds the full output verbatim.
pub(crate) fn assistant_contents(output: Option<&Value>) -> Vec<AssistantContent> {
    let Some(messages) = output.and_then(Value::as_array) else {
        return Vec::new();
    };
    messages
        .iter()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|assistant| {
            let mut text_parts = Vec::new();
            let mut tool_call_parts = Vec::new();
            let mut parts_not_in_turn = 0_u64;
            let parts = assistant
                .get("parts")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice);
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("tool_call") => tool_call_parts.push(part.clone()),
                    Some("text") | None => text_parts.push(part.clone()),
                    Some(_) => parts_not_in_turn += 1,
                }
            }
            AssistantContent {
                message: Value::Array(text_parts),
                tool_calls: if tool_call_parts.is_empty() {
                    None
                } else {
                    Some(Value::Array(tool_call_parts))
                },
                parts_not_in_turn,
            }
        })
        .collect()
}

/// True when a message object has `role == "user"`.
pub(crate) fn is_user(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user")
}

/// Synthesized structured equivalents of a legacy span's message bodies (F8).
#[derive(Debug, Default)]
struct SynthMessages {
    system_instructions: Option<Value>,
    input_messages: Option<Value>,
    output_messages: Option<Value>,
}

/// Reduces a legacy (<= v1.36) span's message events to structured-identical
/// `serde_json::Value`s (the central correctness lever).
///
/// The reduction is byte-identical to the structured path for the supported
/// single-turn text + tool_call shapes whenever legacy bodies are round-trippable
/// JSON. Documented limitations on exact hash-match parity with *real-world*
/// structured emitters:
///
/// - Per-message `finish_reason` is lifted from the `gen_ai.choice` body into the
///   assistant message object (to match the v1.37 per-message field); parity
///   holds for telemetry that agrees on this placement.
/// - Tool-call `arguments` that are JSON strings are parsed to a value so they
///   match the structured object form, but only round-trippable JSON canonicalizes
///   identically.
/// - `gen_ai.tool.message` reduction to a `tool_call_response` part is best-effort.
///
/// Deterministic role -> slot routing: `gen_ai.system.message` -> system; a
/// `gen_ai.user.message` / `gen_ai.tool.message` -> input; a
/// `gen_ai.assistant.message` -> input *only* when a `gen_ai.choice` exists in the
/// same span (then the choice is the response), otherwise the assistant message is
/// the response -> output; `gen_ai.choice` -> output.
fn synthesize_structured_from_legacy(span: &NormalizedSpan) -> SynthMessages {
    let has_choice = span.events_named(semconv::EVENT_CHOICE).next().is_some();

    let mut system_parts: Vec<Value> = Vec::new();
    for event in span.events_named(semconv::EVENT_SYSTEM_MESSAGE) {
        if let Some(content) = event.attr_str(semconv::BODY_CONTENT) {
            if !content.trim().is_empty() {
                system_parts.extend(content_to_parts(content));
            }
        }
    }

    let mut input_messages: Vec<Value> = Vec::new();
    let mut output_messages: Vec<Value> = Vec::new();
    let mut choices: Vec<(i64, usize, Value)> = Vec::new();
    for (event_pos, event) in span.events.iter().enumerate() {
        match event.name.as_str() {
            semconv::EVENT_USER_MESSAGE => {
                input_messages.extend(legacy_user_message(event));
            }
            semconv::EVENT_TOOL_MESSAGE => {
                input_messages.extend(legacy_tool_message(event));
            }
            semconv::EVENT_ASSISTANT_MESSAGE => {
                if let Some(msg) = legacy_assistant_message(event) {
                    if has_choice {
                        input_messages.push(msg);
                    } else {
                        output_messages.push(msg);
                    }
                }
            }
            semconv::EVENT_CHOICE => {
                if let Some(msg) = legacy_choice_message(event) {
                    let index = event.attr_i64(semconv::BODY_INDEX).unwrap_or(0);
                    choices.push((index, event_pos, msg));
                }
            }
            _ => {}
        }
    }
    choices.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    output_messages.extend(choices.into_iter().map(|(_, _, msg)| msg));

    let non_empty = |v: Vec<Value>| (!v.is_empty()).then_some(Value::Array(v));
    SynthMessages {
        system_instructions: non_empty(system_parts),
        input_messages: non_empty(input_messages),
        output_messages: non_empty(output_messages),
    }
}

/// Converts a legacy event-body `content` string into the structured `parts`
/// array shape (`[{"type":"text","content": <string>}]`).
///
/// Pass-through only happens for a JSON array whose elements are objects carrying
/// a `type` (already structured parts); any other shape is wrapped verbatim as a
/// single text part.
fn content_to_parts(content: &str) -> Vec<Value> {
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(content) {
        let already_parts = !items.is_empty()
            && items
                .iter()
                .all(|item| item.get("type").and_then(Value::as_str).is_some());
        if already_parts {
            return items;
        }
    }
    let mut part = Map::new();
    part.insert("type".to_owned(), Value::String("text".to_owned()));
    part.insert("content".to_owned(), Value::String(content.to_owned()));
    vec![Value::Object(part)]
}

/// Builds a `{"role": role, "parts": [..]}` message object.
fn legacy_message_object(role: &str, parts: Vec<Value>) -> Value {
    let mut obj = Map::new();
    obj.insert("role".to_owned(), Value::String(role.to_owned()));
    obj.insert("parts".to_owned(), Value::Array(parts));
    Value::Object(obj)
}

/// Returns a non-blank string body attribute.
fn body<'a>(event: &'a NormalizedEvent, key: &str) -> Option<&'a str> {
    event.attr_str(key).filter(|c| !c.trim().is_empty())
}

/// Reduces a `gen_ai.user.message` event to a user message object.
fn legacy_user_message(event: &NormalizedEvent) -> Option<Value> {
    let content = body(event, semconv::BODY_CONTENT)?;
    let role = event.attr_str(semconv::BODY_ROLE).unwrap_or("user");
    Some(legacy_message_object(role, content_to_parts(content)))
}

/// Reduces a `gen_ai.tool.message` event to a tool message object.
fn legacy_tool_message(event: &NormalizedEvent) -> Option<Value> {
    let content = body(event, semconv::BODY_CONTENT)?;
    let mut part = Map::new();
    part.insert(
        "type".to_owned(),
        Value::String("tool_call_response".to_owned()),
    );
    if let Some(id) = event.attr_str(semconv::BODY_ID) {
        part.insert("id".to_owned(), Value::String(id.to_owned()));
    }
    part.insert("response".to_owned(), parse_json_or_string(content));
    Some(legacy_message_object("tool", vec![Value::Object(part)]))
}

/// Reduces a `gen_ai.assistant.message` event to an assistant message object.
fn legacy_assistant_message(event: &NormalizedEvent) -> Option<Value> {
    let mut parts = body(event, semconv::BODY_CONTENT)
        .map(content_to_parts)
        .unwrap_or_default();
    parts.extend(
        body(event, semconv::BODY_TOOL_CALLS)
            .map(tool_calls_to_parts)
            .unwrap_or_default(),
    );
    (!parts.is_empty()).then(|| legacy_message_object("assistant", parts))
}

/// Reduces a `gen_ai.choice` event to an assistant message object matching the
/// structured `gen_ai.output.messages` element shape.
fn legacy_choice_message(event: &NormalizedEvent) -> Option<Value> {
    let message = event
        .attr_str(semconv::BODY_MESSAGE)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());

    let content_str = message
        .as_ref()
        .and_then(|m| m.get(semconv::BODY_CONTENT))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| event.attr_str(semconv::BODY_CONTENT).map(str::to_owned));

    let mut parts = content_str
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(content_to_parts)
        .unwrap_or_default();

    let tool_calls_value = message
        .as_ref()
        .and_then(|m| m.get(semconv::BODY_TOOL_CALLS))
        .cloned()
        .or_else(|| {
            event
                .attr_str(semconv::BODY_TOOL_CALLS)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        });
    parts.extend(
        tool_calls_value
            .map(|v| tool_calls_value_to_parts(&v))
            .unwrap_or_default(),
    );
    if parts.is_empty() {
        return None;
    }

    let mut obj = Map::new();
    obj.insert("role".to_owned(), Value::String("assistant".to_owned()));
    obj.insert("parts".to_owned(), Value::Array(parts));
    if let Some(finish) = event.attr_str(semconv::BODY_FINISH_REASON) {
        obj.insert("finish_reason".to_owned(), Value::String(finish.to_owned()));
    }
    Some(Value::Object(obj))
}

/// Parses a `tool_calls` JSON-string into structured `tool_call` parts.
fn tool_calls_to_parts(raw: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(raw)
        .map(|value| tool_calls_value_to_parts(&value))
        .unwrap_or_default()
}

/// Converts an OpenAI-style `tool_calls` array value into structured `tool_call`
/// parts (`{"type":"tool_call","id":..,"name":..,"arguments":..}`).
fn tool_calls_value_to_parts(value: &Value) -> Vec<Value> {
    let Some(calls) = value.as_array() else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for call in calls {
        let mut part = Map::new();
        part.insert("type".to_owned(), Value::String("tool_call".to_owned()));
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            part.insert("id".to_owned(), Value::String(id.to_owned()));
        }
        let function = call.get("function");
        if let Some(name) = function.and_then(|f| f.get("name")).and_then(Value::as_str) {
            part.insert("name".to_owned(), Value::String(name.to_owned()));
        }
        if let Some(arguments) = function.and_then(|f| f.get("arguments")) {
            let arguments_value = match arguments {
                Value::String(s) => parse_json_or_string(s),
                other => other.clone(),
            };
            part.insert("arguments".to_owned(), arguments_value);
        }
        parts.push(Value::Object(part));
    }
    parts
}

/// Parses a string as JSON, falling back to the verbatim string on parse error.
fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_content_is_absent() {
        for v in [
            AnyValue::String(String::new()),
            AnyValue::String("   ".to_owned()),
            AnyValue::String("[]".to_owned()),
            AnyValue::String("null".to_owned()),
            AnyValue::String("{}".to_owned()),
            AnyValue::Array(Vec::new()),
            AnyValue::Empty,
        ] {
            assert_eq!(content_value(Some(&v)), None, "{v:?}");
        }
        assert_eq!(
            content_value(Some(&AnyValue::String("rainy".to_owned()))),
            Some(json!("rainy"))
        );
    }

    #[test]
    fn reasoning_parts_are_not_assistant_text() {
        let output = json!([
            {"role":"assistant","parts":[
                {"type":"reasoning","content":"thinking"},
                {"type":"text","content":"answer"}
            ]},
            {"role":"assistant","parts":[{"type":"text","content":"alt"}]}
        ]);
        let turns = assistant_contents(Some(&output));
        assert_eq!(turns.len(), 2);
        assert_eq!(
            turns[0].message,
            json!([{"type":"text","content":"answer"}])
        );
        assert_eq!(turns[0].parts_not_in_turn, 1);
        assert!(turns[0].tool_calls.is_none());
    }
}
