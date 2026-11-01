//! Regression tests for the v3 importer hardening (OTEL-1 .. OTEL-12): current
//! GenAI semconv forms, OTLP/JSON variants, failure recording, trace selection,
//! validation-before-write, size caps and timestamp honesty.

use std::sync::Arc;

use akmon_journal::{
    AttemptStatus, Event, EventKind, Hash, HashAlgorithm, MemoryObjectStore, MemorySessionGraph,
    ObjectStore, RedbObjectStore, SessionGraph,
};
use akmon_otel::{
    CaptureLevel, ImportOptions, ImportReport, OtelImportError, import_otel_genai,
    import_trace_file_into_store, plan_otel_import, read_trace_file,
};
use serde_json::{Value, json};
use uuid::Uuid;

const STRUCTURED_1_37: &str = include_str!("fixtures/semconv_1_37_structured.otlp.json");
const DETAILS_1_42: &str = include_str!("fixtures/semconv_1_42_details_event.otlp.json");
const VARIANTS: &str = include_str!("fixtures/otlp_json_variants_structured.otlp.json");

const T0: &str = "1748000000000000000";
const T1: &str = "1748000001000000000";
const T2: &str = "1748000002000000000";

fn stores() -> (Arc<MemoryObjectStore>, MemorySessionGraph) {
    let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
    let graph = MemorySessionGraph::open_new(Arc::clone(&store), Uuid::new_v4());
    (store, graph)
}

/// A successful import: report, session history, and the object store.
type Imported = (ImportReport, Vec<(Hash, Event)>, Arc<MemoryObjectStore>);

fn import(trace: &str) -> Result<Imported, OtelImportError> {
    let (store, mut graph) = stores();
    let report = import_otel_genai(trace.as_bytes(), store.as_ref(), &mut graph)?;
    let history = graph.history().unwrap_or_else(|e| panic!("history: {e}"));
    let verify = graph.verify().unwrap_or_else(|e| panic!("verify: {e}"));
    assert!(verify.is_clean(), "session must verify clean: {verify:?}");
    Ok((report, history, store))
}

fn import_err(trace: &str) -> OtelImportError {
    match import(trace) {
        Ok(_) => panic!("import unexpectedly succeeded"),
        Err(err) => err,
    }
}

fn import_ok(trace: &str) -> Imported {
    import(trace).unwrap_or_else(|e| panic!("import failed: {e}"))
}

fn object(store: &MemoryObjectStore, hash: &Hash) -> Value {
    let bytes = store
        .get(hash)
        .unwrap_or_else(|e| panic!("get: {e}"))
        .unwrap_or_else(|| panic!("missing object"));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("json: {e}"))
}

fn config(history: &[(Hash, Event)], store: &MemoryObjectStore) -> Value {
    match &history[0].1.kind {
        EventKind::SessionStart { config_hash, .. } => object(store, config_hash),
        other => panic!("first event must be SessionStart, got {other:?}"),
    }
}

/// Every content hash, by event, in order (excluding the SessionStart objects).
fn content_hashes(history: &[(Hash, Event)]) -> Vec<Hash> {
    let mut out = Vec::new();
    for (_, event) in history {
        match &event.kind {
            EventKind::UserTurn { prompt_hash } => out.push(prompt_hash.clone()),
            EventKind::AssistantTurn {
                message_hash,
                tool_calls_hash,
            } => {
                out.push(message_hash.clone());
                out.extend(tool_calls_hash.clone());
            }
            EventKind::ProviderCall { attempts, .. } => {
                for a in attempts {
                    out.push(a.request_hash.clone());
                    out.extend(a.response_hash.clone());
                }
            }
            EventKind::ToolCall {
                input_hash,
                output_hash,
                ..
            } => {
                out.push(input_hash.clone());
                out.push(output_hash.clone());
            }
            _ => {}
        }
    }
    out
}

fn provider_attempts(history: &[(Hash, Event)]) -> Vec<akmon_journal::AttemptRecord> {
    history
        .iter()
        .filter_map(|(_, e)| match &e.kind {
            EventKind::ProviderCall { attempts, .. } => attempts.first().cloned(),
            _ => None,
        })
        .collect()
}

/// Builds a one-resource OTLP/JSON trace from span objects.
fn trace(spans: &[Value]) -> String {
    json!({"resourceSpans":[{"scopeSpans":[{"spans": spans}]}]}).to_string()
}

fn s(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

/// A span with the given ids/op and extra attributes, timestamps T0..T1.
fn span(trace_id: &str, span_id: &str, op: &str, extra: Vec<Value>) -> Value {
    let mut attrs = vec![s("gen_ai.operation.name", op)];
    attrs.extend(extra);
    json!({
        "traceId": trace_id, "spanId": span_id, "parentSpanId": "",
        "startTimeUnixNano": T0, "endTimeUnixNano": T1, "attributes": attrs
    })
}

fn user_msg(text: &str) -> Value {
    json!({"role":"user","parts":[{"type":"text","content": text}]})
}

fn assistant_msg(text: &str) -> Value {
    json!({"role":"assistant","parts":[{"type":"text","content": text}]})
}

// ---------------------------------------------------------------- OTEL-1 ----

#[test]
fn content_only_on_invoke_agent_is_structural() {
    let t = trace(&[
        span(
            "t1",
            "a1",
            "invoke_agent",
            vec![s(
                "gen_ai.input.messages",
                &json!([user_msg("hi")]).to_string(),
            )],
        ),
        span("t1", "c1", "chat", vec![]),
    ]);
    let (report, _, _) = import_ok(&t);
    assert_eq!(report.capture_level, CaptureLevel::Structural);
    assert_eq!(report.turns_emitted, 0);
}

#[test]
fn empty_content_strings_are_structural() {
    let t = trace(&[span(
        "t1",
        "c1",
        "chat",
        vec![
            s("gen_ai.input.messages", ""),
            s("gen_ai.output.messages", "   "),
            s("gen_ai.system_instructions", "[]"),
        ],
    )]);
    let (report, history, store) = import_ok(&t);
    assert_eq!(report.capture_level, CaptureLevel::Structural);
    let attempt = &provider_attempts(&history)[0];
    assert_eq!(
        object(&store, &attempt.request_hash).get("akmon_otel_metadata"),
        Some(&json!(true))
    );
}

#[test]
fn some_but_not_all_slots_is_partial_and_fails_full() {
    // Chat span with real request + response, tool span with no captured content.
    let t = trace(&[
        span(
            "t1",
            "c1",
            "chat",
            vec![
                s(
                    "gen_ai.input.messages",
                    &json!([user_msg("hi")]).to_string(),
                ),
                s(
                    "gen_ai.output.messages",
                    &json!([assistant_msg("hello")]).to_string(),
                ),
            ],
        ),
        span("t1", "x1", "execute_tool", vec![s("gen_ai.tool.name", "t")]),
    ]);
    let (report, history, store) = import_ok(&t);
    assert_eq!(report.capture_level, CaptureLevel::Partial);
    assert_eq!(
        (report.content_slots_real, report.content_slots_total),
        (2, 4)
    );
    assert_eq!(
        config(&history, &store).get("capture_level"),
        Some(&json!("partial"))
    );
}

#[test]
fn structured_any_value_content_is_full_and_hashes_like_json_string_form() {
    let (report_v, history_v, _) = import_ok(VARIANTS);
    let (report_a, history_a, _) = import_ok(STRUCTURED_1_37);
    assert_eq!(report_v.capture_level, CaptureLevel::Full);
    assert_eq!(report_a.capture_level, CaptureLevel::Full);
    assert_eq!(content_hashes(&history_v), content_hashes(&history_a));
}

// ----------------------------------------------------------- OTEL-2 / 3 ----

#[test]
fn inference_details_event_is_content_and_matches_attribute_form() {
    let (report_d, history_d, store_d) = import_ok(DETAILS_1_42);
    let (_, history_a, _) = import_ok(STRUCTURED_1_37);
    assert_eq!(report_d.capture_level, CaptureLevel::Full);
    assert_eq!(content_hashes(&history_d), content_hashes(&history_a));
    // Tool arguments/result are span attributes; the chat content is the event.
    assert_eq!(
        report_d.source_form,
        "structured_attributes+inference_details_event"
    );
    assert_eq!(report_d.source_semconv, "1.42.0");

    // The evaluation event is recorded as structural metadata in the signed config.
    assert_eq!(report_d.structural_events_recorded, 1);
    let cfg = config(&history_d, &store_d);
    let events = cfg
        .get("structural_events")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("structural_events missing: {cfg}"));
    assert_eq!(
        events[0].get("name"),
        Some(&json!("gen_ai.evaluation.result"))
    );
    assert_eq!(
        events[0].pointer("/attributes/gen_ai.evaluation.score.value"),
        Some(&json!(0.92))
    );
    assert_eq!(cfg.get("source_semconv"), Some(&json!("1.42.0")));
    assert_eq!(
        cfg.pointer("/agent_identity/name"),
        Some(&json!("weather-agent"))
    );
}

#[test]
fn undeclared_schema_is_recorded_honestly() {
    let (report, history, store) = import_ok(STRUCTURED_1_37);
    assert_eq!(report.source_semconv, "undeclared");
    assert_eq!(report.source_form, "structured_attributes");
    let cfg = config(&history, &store);
    assert_eq!(cfg.get("source_semconv"), Some(&json!("undeclared")));
    assert_eq!(
        cfg.get("importer_semconv"),
        Some(&json!(akmon_otel::SEMCONV_VERSION))
    );
}

#[test]
fn unknown_non_legacy_gen_ai_event_is_recorded_not_refused() {
    let mut chat = span("t1", "c1", "chat", vec![]);
    chat["events"] = json!([{"name":"gen_ai.some_future.event","attributes":[s("k","v")]}]);
    let (report, _, _) = import_ok(&trace(&[chat]));
    assert_eq!(report.structural_events_recorded, 1);
}

#[test]
fn unknown_legacy_message_event_is_refused_with_its_name() {
    let mut chat = span("t1", "c1", "chat", vec![]);
    chat["events"] = json!([{"name":"gen_ai.developer.message"}]);
    let err = import_err(&trace(&[chat]));
    match err {
        OtelImportError::LegacySemconvUnsupported(name) => {
            assert_eq!(name, "gen_ai.developer.message");
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn new_operation_names_are_recognised() {
    for op in ["text_completion", "generate_content", "embeddings"] {
        let (report, _, _) = import_ok(&trace(&[span("t1", "c1", op, vec![])]));
        assert_eq!(report.provider_calls, 1, "{op}");
    }
    let t = trace(&[
        span("t1", "w1", "invoke_workflow", vec![]),
        span("t1", "g1", "create_agent", vec![]),
        span("t1", "x1", "execute_tool", vec![s("gen_ai.tool.name", "t")]),
    ]);
    let (report, _, _) = import_ok(&t);
    assert_eq!((report.tool_calls, report.spans_skipped), (1, 2));
}

// ---------------------------------------------------------------- OTEL-4 ----

#[test]
fn otlp_json_variants_import() {
    let (report, history, store) = import_ok(VARIANTS);
    assert_eq!(report.provider_calls, 1);
    let attempt = &provider_attempts(&history)[0];
    assert_eq!(attempt.started_at.unix_timestamp(), 1_748_000_000);
    assert!(matches!(attempt.status, AttemptStatus::Success));
    assert!(
        object(&store, &attempt.request_hash)
            .get("input_messages")
            .is_some()
    );
}

// ---------------------------------------------------------------- OTEL-5 ----

#[test]
fn span_status_error_is_recorded_as_failure() {
    let mut chat = span("t1", "c1", "chat", vec![]);
    chat["status"] = json!({"code": 2, "message": "boom"});
    let (report, history, _) = import_ok(&trace(&[chat]));
    assert_eq!(report.failed_calls, 1);
    let attempt = &provider_attempts(&history)[0];
    assert!(matches!(&attempt.status, AttemptStatus::Other(l) if l == "span_status_error"));
    assert_eq!(attempt.error_message.as_deref(), Some("boom"));
}

#[test]
fn exception_event_is_recorded_as_failure() {
    let mut chat = span("t1", "c1", "chat", vec![]);
    chat["events"] = json!([{"name":"exception","attributes":[s("exception.type","TimeoutError"), s("exception.message","timed out")]}]);
    let (_, history, _) = import_ok(&trace(&[chat]));
    let attempt = &provider_attempts(&history)[0];
    assert!(matches!(&attempt.status, AttemptStatus::Other(l) if l == "TimeoutError"));
    assert_eq!(attempt.error_message.as_deref(), Some("timed out"));
}

#[test]
fn failed_tool_records_error_in_output_object() {
    let mut tool = span(
        "t1",
        "x1",
        "execute_tool",
        vec![
            s("gen_ai.tool.name", "get_weather"),
            s("error.type", "ToolTimeout"),
        ],
    );
    tool["status"] = json!({"code": "STATUS_CODE_ERROR", "message": "no reply"});
    let (report, history, store) = import_ok(&trace(&[tool]));
    assert_eq!(report.failed_calls, 1);
    let output = history
        .iter()
        .find_map(|(_, e)| match &e.kind {
            EventKind::ToolCall { output_hash, .. } => Some(object(&store, output_hash)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no tool call"));
    assert_eq!(output.get("akmon_otel_tool_error"), Some(&json!(true)));
    assert_eq!(
        output.pointer("/error/error_type"),
        Some(&json!("ToolTimeout"))
    );
    assert_eq!(output.pointer("/error/message"), Some(&json!("no reply")));
}

// ---------------------------------------------------------------- OTEL-6 ----

#[test]
fn identical_duplicate_spans_are_dropped() {
    let chat = span("t1", "c1", "chat", vec![]);
    let (report, _, _) = import_ok(&trace(&[chat.clone(), chat]));
    assert_eq!(
        (report.provider_calls, report.duplicate_spans_dropped),
        (1, 1)
    );
}

#[test]
fn differing_duplicate_span_ids_are_rejected() {
    let a = span("t1", "c1", "chat", vec![]);
    let b = span("t1", "c1", "chat", vec![s("gen_ai.request.model", "other")]);
    let err = import_err(&trace(&[a, b]));
    assert!(
        matches!(err, OtelImportError::DuplicateSpanId { .. }),
        "{err:?}"
    );
}

#[test]
fn multiple_traces_require_a_selector() {
    let t = trace(&[
        span("aa", "c1", "chat", vec![]),
        span("bb", "c2", "chat", vec![]),
    ]);
    let err = import_err(&t);
    assert!(
        matches!(err, OtelImportError::MultipleTraces { count: 2, .. }),
        "{err:?}"
    );

    let options = ImportOptions {
        trace_id: Some("BB".to_owned()),
        ..ImportOptions::default()
    };
    let (store, mut graph) = stores();
    let report = plan_otel_import(t.as_bytes(), &options)
        .and_then(|p| p.emit(store.as_ref(), &mut graph))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.provider_calls, 1);
    assert_eq!(report.trace_id.as_deref(), Some("bb"));

    let missing = ImportOptions {
        trace_id: Some("cc".to_owned()),
        ..ImportOptions::default()
    };
    let err = plan_otel_import(t.as_bytes(), &missing).expect_err("must reject");
    assert!(
        matches!(err, OtelImportError::TraceIdNotFound { .. }),
        "{err:?}"
    );
}

#[test]
fn parent_precedes_child_with_equal_start() {
    // Child id sorts before parent id; same start nanosecond. Parent must come first.
    let parent = span(
        "t1",
        "ff",
        "chat",
        vec![s("gen_ai.provider.name", "parent")],
    );
    let mut child = span("t1", "00", "execute_tool", vec![s("gen_ai.tool.name", "t")]);
    child["parentSpanId"] = json!("ff");
    let (_, history, _) = import_ok(&trace(&[child, parent]));
    let kinds: Vec<&str> = history
        .iter()
        .filter_map(|(_, e)| match &e.kind {
            EventKind::ProviderCall { .. } => Some("provider"),
            EventKind::ToolCall { .. } => Some("tool"),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["provider", "tool"]);
}

// ---------------------------------------------------------------- OTEL-7 ----

#[test]
fn no_genai_spans_is_an_error() {
    let http = json!({"traceId":"t1","spanId":"h1","name":"GET /","startTimeUnixNano":T0,"endTimeUnixNano":T1});
    let err = import_err(&trace(&[http]));
    assert!(
        matches!(err, OtelImportError::NoGenAiSpans { spans: 1 }),
        "{err:?}"
    );

    let agent_only = trace(&[span("t1", "a1", "invoke_agent", vec![])]);
    let err = import_err(&agent_only);
    assert!(
        matches!(err, OtelImportError::NoGenAiSpans { .. }),
        "{err:?}"
    );
}

// ---------------------------------------------------------------- OTEL-8 ----

#[test]
fn failed_plan_writes_nothing() {
    let (store, mut graph) = stores();
    let bad = trace(&[
        span("aa", "c1", "chat", vec![]),
        span("bb", "c2", "chat", vec![]),
    ]);
    assert!(plan_otel_import(bad.as_bytes(), &ImportOptions::default()).is_err());
    let err = import_otel_genai(bad.as_bytes(), store.as_ref(), &mut graph).expect_err("refused");
    assert!(err.is_input_error());
    assert!(graph.history().unwrap_or_else(|e| panic!("{e}")).is_empty());
}

#[test]
fn trace_file_into_redb_store_round_trip_and_refusal() {
    let dir = std::env::temp_dir().join(format!("akmon-otel-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let store = Arc::new(
        RedbObjectStore::create(&dir.join("journal.redb"), HashAlgorithm::Sha256)
            .unwrap_or_else(|e| panic!("{e}")),
    );
    let good = dir.join("good.json");
    std::fs::write(&good, STRUCTURED_1_37).unwrap_or_else(|e| panic!("{e}"));
    let report = import_trace_file_into_store(&good, Arc::clone(&store), &ImportOptions::default())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.capture_level, CaptureLevel::Full);
    assert!(akmon_journal::session_head_row_exists(&store, report.session_id).unwrap_or(false));

    let tiny = ImportOptions {
        max_bytes: 10,
        ..ImportOptions::default()
    };
    let err = import_trace_file_into_store(&good, Arc::clone(&store), &tiny).expect_err("cap");
    assert!(
        matches!(err, OtelImportError::TooLarge { max: 10, .. }),
        "{err:?}"
    );
    let err =
        import_trace_file_into_store(&dir.join("missing.json"), store, &ImportOptions::default())
            .expect_err("io");
    assert!(matches!(err, OtelImportError::Io(_)), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- OTEL-9 ----

#[test]
fn size_cap_applies_to_bytes_and_files() {
    let options = ImportOptions {
        max_bytes: 16,
        ..ImportOptions::default()
    };
    let err = plan_otel_import(STRUCTURED_1_37.as_bytes(), &options).expect_err("cap");
    assert!(matches!(err, OtelImportError::TooLarge { max: 16, .. }));
    let path = std::env::temp_dir().join(format!("akmon-otel-cap-{}.json", Uuid::new_v4()));
    std::fs::write(&path, STRUCTURED_1_37).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        read_trace_file(&path, 16),
        Err(OtelImportError::TooLarge { .. })
    ));
    assert!(read_trace_file(&path, 1 << 20).is_ok());
    let _ = std::fs::remove_file(&path);
}

// --------------------------------------------------------------- OTEL-10 ----

#[test]
fn invalid_timestamps_are_rejected_never_1970() {
    let cases = [
        (json!(null), json!(T1), "missing"),
        (json!("-5"), json!(T1), "negative"),
        (json!("0"), json!(T1), "unset"),
        (json!("abc"), json!(T1), "integer"),
        (json!(T2), json!(T1), "before"),
    ];
    for (start, end, needle) in cases {
        let mut chat = span("t1", "c1", "chat", vec![]);
        chat["startTimeUnixNano"] = start;
        chat["endTimeUnixNano"] = end;
        let err = import_err(&trace(&[chat]));
        match err {
            OtelImportError::InvalidTimestamp { reason, .. } => {
                assert!(reason.contains(needle), "{reason} should mention {needle}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

// --------------------------------------------------------------- OTEL-12 ----

#[test]
fn every_new_user_message_becomes_a_user_turn() {
    let first = json!([user_msg("q1")]);
    let second = json!([user_msg("q1"), assistant_msg("a1"), user_msg("q2")]);
    let mut c2 = span(
        "t1",
        "c2",
        "chat",
        vec![
            s("gen_ai.input.messages", &second.to_string()),
            s(
                "gen_ai.output.messages",
                &json!([{"role":"assistant","parts":[{"type":"reasoning","content":"hmm"},{"type":"text","content":"a2"}]}]).to_string(),
            ),
        ],
    );
    c2["startTimeUnixNano"] = json!(T1);
    c2["endTimeUnixNano"] = json!(T2);
    let t = trace(&[
        span(
            "t1",
            "c1",
            "chat",
            vec![
                s("gen_ai.input.messages", &first.to_string()),
                s(
                    "gen_ai.output.messages",
                    &json!([assistant_msg("a1")]).to_string(),
                ),
            ],
        ),
        c2,
    ]);
    let (report, history, store) = import_ok(&t);
    let prompts: Vec<Value> = history
        .iter()
        .filter_map(|(_, e)| match &e.kind {
            EventKind::UserTurn { prompt_hash } => Some(object(&store, prompt_hash)),
            _ => None,
        })
        .collect();
    assert_eq!(prompts, vec![user_msg("q1"), user_msg("q2")]);
    assert_eq!(report.non_text_parts_omitted, 1);
    assert_eq!(report.user_messages_omitted, 0);
    assert_eq!(report.turns_emitted, 4);
}
