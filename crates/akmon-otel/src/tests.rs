//! Unit tests for the importer (moved from lib.rs).

use super::*;
use akmon_journal::{Event, Hash, HashAlgorithm, MemoryObjectStore, MemorySessionGraph};
use serde_json::Value;
use std::sync::Arc;

const FIXTURE_A: &[u8] = br#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}},{"key":"gen_ai.system_instructions","value":{"stringValue":"[{\"type\":\"text\",\"content\":\"You are a helpful weather assistant.\"}]"}},{"key":"gen_ai.input.messages","value":{"stringValue":"[{\"role\":\"user\",\"parts\":[{\"type\":\"text\",\"content\":\"Weather in Paris?\"}]}]"}},{"key":"gen_ai.output.messages","value":{"stringValue":"[{\"role\":\"assistant\",\"parts\":[{\"type\":\"tool_call\",\"id\":\"call_x\",\"name\":\"get_weather\",\"arguments\":{\"location\":\"Paris\"}}],\"finish_reason\":\"tool_calls\"}]"}}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}},{"key":"gen_ai.tool.call.arguments","value":{"stringValue":"{\"location\":\"Paris\"}"}},{"key":"gen_ai.tool.call.result","value":{"stringValue":"rainy, 57F"}}]}]}]}]}"#;

const FIXTURE_B: &[u8] = br#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}}]}]}]}]}"#;

const FIXTURE_LEGACY: &[u8] = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"abcd","spanId":"1111","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}}],"events":[{"name":"gen_ai.user.message"}]}]}]}]}"#;

// FIXTURE_C: the legacy (<= v1.36) message-event form of the SAME logical
// session as FIXTURE_A. Same trace/span ids, same provider/model/conversation/
// usage attributes, identical `execute_tool` span. The chat span carries NO
// structured content attributes; instead it carries gen_ai.* message events:
//   - gen_ai.system.message  content="You are a helpful weather assistant."
//   - gen_ai.user.message    content="Weather in Paris?"
//   - gen_ai.choice          index=0, finish_reason="tool_calls", message=
//       {"tool_calls":[{"id":"call_x","type":"function","function":
//         {"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]}
const FIXTURE_C: &[u8] = br#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}}],"events":[{"name":"gen_ai.system.message","attributes":[{"key":"content","value":{"stringValue":"You are a helpful weather assistant."}}]},{"name":"gen_ai.user.message","attributes":[{"key":"content","value":{"stringValue":"Weather in Paris?"}}]},{"name":"gen_ai.choice","attributes":[{"key":"index","value":{"intValue":"0"}},{"key":"finish_reason","value":{"stringValue":"tool_calls"}},{"key":"message","value":{"stringValue":"{\"tool_calls\":[{\"id\":\"call_x\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"location\\\":\\\"Paris\\\"}\"}}]}"}}]}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}},{"key":"gen_ai.tool.call.arguments","value":{"stringValue":"{\"location\":\"Paris\"}"}},{"key":"gen_ai.tool.call.result","value":{"stringValue":"rainy, 57F"}}]}]}]}]}"#;

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
    assert_eq!(report.semconv_version, SEMCONV_VERSION);

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
fn unknown_legacy_event_is_rejected() {
    // An UNSUPPORTED legacy gen_ai.* event (not one of the five forms) is
    // refused: we never silently drop legacy content we cannot reduce (F8).
    let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"abcd","spanId":"1111","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}}],"events":[{"name":"gen_ai.developer.message"}]}]}]}]}"#;
    let (store, mut graph) = stores();
    let err = import_otel_genai(json, store.as_ref(), &mut graph)
        .expect_err("unrecognized legacy event must be rejected");
    assert!(matches!(err, OtelImportError::LegacySemconvUnsupported(_)));
}

#[test]
fn bodiless_supported_legacy_event_imports_as_structural() {
    // A supported legacy event with NO body (FIXTURE_LEGACY's bodiless
    // gen_ai.user.message) is NOT refused; it imports as Structural because it
    // carries no real content.
    let (store, mut graph) = stores();
    let report =
        import_otel_genai(FIXTURE_LEGACY, store.as_ref(), &mut graph).unwrap_or_else(|_| panic!());
    assert_eq!(report.capture_level, CaptureLevel::Structural);
    let verify = graph.verify().unwrap_or_else(|_| unreachable!());
    assert!(verify.is_clean(), "session must verify clean: {verify:?}");
}

#[test]
fn deprecated_system_imports_as_structural() {
    // Under the new policy `gen_ai.system` alone (provider identity, no legacy
    // events) is NO LONGER refused: it imports as a Structural session with
    // provider = "openai" recorded in the config object.
    let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"a","spanId":"1","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.system","value":{"stringValue":"openai"}}]}]}]}]}"#;
    let (store, mut graph) = stores();
    let report = import_otel_genai(json, store.as_ref(), &mut graph).unwrap_or_else(|_| panic!());
    assert_eq!(report.capture_level, CaptureLevel::Structural);
    assert_eq!(report.provider_calls, 1);

    let history = graph.history().unwrap_or_else(|_| unreachable!());
    // The config object (SessionStart) records provider = "openai".
    let (_, start_event) = history.first().unwrap_or_else(|| unreachable!());
    let EventKind::SessionStart { config_hash, .. } = &start_event.kind else {
        unreachable!()
    };
    let bytes = store
        .get(config_hash)
        .unwrap_or_else(|_| unreachable!())
        .unwrap_or_else(|| unreachable!());
    let config: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| unreachable!());
    assert_eq!(
        config.get("provider").and_then(Value::as_str),
        Some("openai")
    );

    let verify = graph.verify().unwrap_or_else(|_| unreachable!());
    assert!(verify.is_clean(), "session must verify clean: {verify:?}");
}

#[test]
fn multiple_conversation_ids_rejected() {
    let json = br#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"a","spanId":"1","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.input.messages","value":{"stringValue":"[]"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-a"}}]},{"traceId":"a","spanId":"2","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"3","endTimeUnixNano":"4","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-b"}}]}]}]}]}"#;
    let (store, mut graph) = stores();
    let err = import_otel_genai(json, store.as_ref(), &mut graph)
        .expect_err("multiple conversation ids must be rejected");
    assert!(matches!(err, OtelImportError::MultipleSessions(_)));
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
        crate::canonical::canonical_json_bytes(&a).unwrap_or_else(|_| unreachable!()),
        crate::canonical::canonical_json_bytes(&b).unwrap_or_else(|_| unreachable!())
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

/// The content-object hashes carrying real content, by slot, for hash-match
/// comparison. Excludes the SessionStart config/cwd hashes (compared
/// separately and not part of the content-equality property).
#[derive(Debug, Default, PartialEq, Eq)]
struct ContentSlots {
    user_prompt: Option<Hash>,
    assistant_message: Option<Hash>,
    assistant_tool_calls: Option<Hash>,
    provider_request: Option<Hash>,
    provider_response: Option<Hash>,
    tool_input: Option<Hash>,
    tool_output: Option<Hash>,
}

fn content_slots(history: &[(Hash, Event)]) -> ContentSlots {
    let mut slots = ContentSlots::default();
    for (_, event) in history {
        match &event.kind {
            EventKind::UserTurn { prompt_hash } => {
                slots.user_prompt = Some(prompt_hash.clone());
            }
            EventKind::AssistantTurn {
                message_hash,
                tool_calls_hash,
            } => {
                slots.assistant_message = Some(message_hash.clone());
                slots.assistant_tool_calls = tool_calls_hash.clone();
            }
            EventKind::ProviderCall { attempts, .. } => {
                if let Some(attempt) = attempts.first() {
                    slots.provider_request = Some(attempt.request_hash.clone());
                    slots.provider_response = attempt.response_hash.clone();
                }
            }
            EventKind::ToolCall {
                input_hash,
                output_hash,
                ..
            } => {
                slots.tool_input = Some(input_hash.clone());
                slots.tool_output = Some(output_hash.clone());
            }
            _ => {}
        }
    }
    slots
}

#[test]
fn fixture_c_legacy_matches_fixture_a_content_hashes() {
    // The load-bearing property: legacy (FIXTURE_C) and structured (FIXTURE_A)
    // telemetry describing the SAME logical content produce the SAME
    // content-object hashes, because both route through the same
    // real_content_bytes / canonical_json_bytes path over structurally-equal
    // synthesized values.
    let (store_a, mut graph_a) = stores();
    let report_a =
        import_otel_genai(FIXTURE_A, store_a.as_ref(), &mut graph_a).unwrap_or_else(|_| panic!());
    let (store_c, mut graph_c) = stores();
    let report_c =
        import_otel_genai(FIXTURE_C, store_c.as_ref(), &mut graph_c).unwrap_or_else(|_| panic!());

    // Both must be Full-capture, one provider call, one tool call.
    assert_eq!(report_a.capture_level, CaptureLevel::Full);
    assert_eq!(report_c.capture_level, CaptureLevel::Full);
    assert_eq!(report_a.provider_calls, 1);
    assert_eq!(report_c.provider_calls, 1);
    assert_eq!(report_a.tool_calls, 1);
    assert_eq!(report_c.tool_calls, 1);

    let history_a = graph_a.history().unwrap_or_else(|_| unreachable!());
    let history_c = graph_c.history().unwrap_or_else(|_| unreachable!());
    let slots_a = content_slots(&history_a);
    let slots_c = content_slots(&history_c);

    // Slot-by-slot equality (stronger than a sorted multiset compare).
    assert_eq!(
        slots_a.user_prompt, slots_c.user_prompt,
        "UserTurn.prompt_hash must match"
    );
    assert_eq!(
        slots_a.assistant_message, slots_c.assistant_message,
        "AssistantTurn.message_hash must match"
    );
    assert_eq!(
        slots_a.assistant_tool_calls, slots_c.assistant_tool_calls,
        "AssistantTurn.tool_calls_hash must match"
    );
    assert_eq!(
        slots_a.provider_request, slots_c.provider_request,
        "ProviderCall request_hash must match"
    );
    assert_eq!(
        slots_a.provider_response, slots_c.provider_response,
        "ProviderCall response_hash must match"
    );
    assert_eq!(
        slots_a.tool_input, slots_c.tool_input,
        "ToolCall input_hash must match"
    );
    assert_eq!(
        slots_a.tool_output, slots_c.tool_output,
        "ToolCall output_hash must match"
    );
    // Whole-struct equality as a belt-and-braces guard.
    assert_eq!(slots_a, slots_c);

    // Every content slot is actually populated (not all-None matching).
    assert!(slots_c.user_prompt.is_some());
    assert!(slots_c.assistant_tool_calls.is_some());
    assert!(slots_c.provider_request.is_some());
    assert!(slots_c.provider_response.is_some());
    assert!(slots_c.tool_input.is_some());
    assert!(slots_c.tool_output.is_some());

    let verify = graph_c.verify().unwrap_or_else(|_| unreachable!());
    assert!(
        verify.is_clean(),
        "legacy session must verify clean: {verify:?}"
    );
}

#[test]
fn fixture_c_legacy_full_capture_emits_all_turns_and_verifies() {
    let (store, mut graph) = stores();
    let report =
        import_otel_genai(FIXTURE_C, store.as_ref(), &mut graph).unwrap_or_else(|_| panic!());

    assert_eq!(report.capture_level, CaptureLevel::Full);
    assert_eq!(report.provider_calls, 1);
    assert_eq!(report.tool_calls, 1);

    let history = graph.history().unwrap_or_else(|_| unreachable!());
    let kinds = kinds(&history);

    assert!(matches!(
        kinds.first(),
        Some(EventKind::SessionStart { .. })
    ));
    assert!(matches!(kinds.last(), Some(EventKind::SessionEnd { .. })));
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
    assert!(kinds.iter().any(|k| matches!(
        k,
        EventKind::AssistantTurn {
            tool_calls_hash: Some(_),
            ..
        }
    )));
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, EventKind::ToolCall { .. }))
    );

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

    let verify = graph.verify().unwrap_or_else(|_| unreachable!());
    assert!(verify.is_clean(), "session must verify clean: {verify:?}");
}

/// Builds a one-trace OTLP/JSON document from raw span JSON objects.
fn trace_doc(spans: &[String]) -> Vec<u8> {
    format!(
        r#"{{"resourceSpans":[{{"scopeSpans":[{{"spans":[{}]}}]}}]}}"#,
        spans.join(",")
    )
    .into_bytes()
}

/// A span JSON object with the given ids, times and `(key, stringValue)` attributes.
fn span_json(trace: &str, span: &str, start: u64, attrs: &[(&str, &str)], events: &str) -> String {
    let attrs: Vec<String> = attrs
        .iter()
        .map(|(k, v)| {
            format!(
                r#"{{"key":{},"value":{{"stringValue":{}}}}}"#,
                serde_json::to_string(k).unwrap_or_default(),
                serde_json::to_string(v).unwrap_or_default()
            )
        })
        .collect();
    format!(
        r#"{{"traceId":"{trace}","spanId":"{span}","parentSpanId":"","name":"s","kind":3,"startTimeUnixNano":"{start}","endTimeUnixNano":"{}","attributes":[{}],"events":[{events}]}}"#,
        start + 1,
        attrs.join(",")
    )
}

const USER_IN: &str = r#"[{"role":"user","parts":[{"type":"text","content":"hi"}]}]"#;
const ASSISTANT_OUT: &str = r#"[{"role":"assistant","parts":[{"type":"text","content":"hello"}]}]"#;

#[test]
fn embeddings_spans_add_no_content_slots() {
    let doc = trace_doc(&[
        span_json(
            "aa",
            "01",
            10,
            &[
                ("gen_ai.operation.name", "chat"),
                ("gen_ai.input.messages", USER_IN),
                ("gen_ai.output.messages", ASSISTANT_OUT),
            ],
            "",
        ),
        span_json(
            "aa",
            "02",
            20,
            &[("gen_ai.operation.name", "embeddings")],
            "",
        ),
    ]);
    let plan = plan_otel_import(&doc, &ImportOptions::default()).expect("plan");
    assert_eq!(plan.provider_calls(), 2);
    assert_eq!(plan.capture_level(), CaptureLevel::Full);

    let only_embeddings = trace_doc(&[span_json(
        "aa",
        "02",
        20,
        &[("gen_ai.operation.name", "embeddings")],
        "",
    )]);
    let plan = plan_otel_import(&only_embeddings, &ImportOptions::default()).expect("plan");
    assert_eq!(plan.capture_level(), CaptureLevel::Structural);
}

#[test]
fn system_instructions_alone_do_not_make_the_request_slot_real() {
    let doc = trace_doc(&[span_json(
        "aa",
        "01",
        10,
        &[
            ("gen_ai.operation.name", "chat"),
            (
                "gen_ai.system_instructions",
                r#"[{"type":"text","content":"be terse"}]"#,
            ),
            ("gen_ai.output.messages", ASSISTANT_OUT),
        ],
        "",
    )]);
    let plan = plan_otel_import(&doc, &ImportOptions::default()).expect("plan");
    assert_eq!(plan.capture_level(), CaptureLevel::Partial);
}

#[test]
fn assistant_choice_without_text_or_tool_calls_is_suppressed() {
    let doc = trace_doc(&[span_json(
        "aa",
        "01",
        10,
        &[
            ("gen_ai.operation.name", "chat"),
            ("gen_ai.input.messages", USER_IN),
            (
                "gen_ai.output.messages",
                r#"[{"role":"assistant","parts":[{"type":"reasoning","content":"thinking"}]}]"#,
            ),
        ],
        "",
    )]);
    let (store, mut graph) = stores();
    let report = import_otel_genai(&doc, store.as_ref(), &mut graph).expect("import");
    assert_eq!(report.turns_suppressed_no_content, 1);
    let history = graph.history().unwrap_or_else(|_| unreachable!());
    assert!(
        !kinds(&history)
            .iter()
            .any(|k| matches!(k, EventKind::AssistantTurn { .. })),
        "no AssistantTurn for a reasoning-only choice"
    );
}

#[test]
fn trace_selection_happens_before_unknown_legacy_refusal() {
    let doc = trace_doc(&[
        span_json(
            "aa",
            "01",
            10,
            &[
                ("gen_ai.operation.name", "chat"),
                ("gen_ai.input.messages", USER_IN),
                ("gen_ai.output.messages", ASSISTANT_OUT),
            ],
            "",
        ),
        span_json(
            "bb",
            "02",
            20,
            &[("gen_ai.operation.name", "chat")],
            r#"{"name":"gen_ai.developer.message"}"#,
        ),
    ]);
    let options = ImportOptions {
        trace_id: Some("aa".to_owned()),
        ..ImportOptions::default()
    };
    let plan = plan_otel_import(&doc, &options).expect("unselected trace must not refuse");
    assert_eq!(plan.provider_calls(), 1);
    let options = ImportOptions {
        trace_id: Some("bb".to_owned()),
        ..ImportOptions::default()
    };
    assert!(matches!(
        plan_otel_import(&doc, &options),
        Err(OtelImportError::LegacySemconvUnsupported(_))
    ));
}

#[test]
fn metadata_envelope_records_importer_semconv_not_source() {
    let bytes = objects::metadata_envelope_bytes("request", serde_json::Map::new());
    let value: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(value.get("source_semconv"), None);
    assert_eq!(
        value.get("importer_semconv").and_then(Value::as_str),
        Some(SEMCONV_VERSION)
    );
}
