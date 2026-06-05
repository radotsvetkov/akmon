//! Integration tests for `akmon otel import` (Item 9.1).
//!
//! Drives the real `akmon` binary end to end: a producer-agnostic OTLP/JSON
//! OpenTelemetry GenAI trace becomes an AGEF session, and (in the roundtrip test)
//! a signed, independently-verifiable AGEF bundle — all through the CLI.

use std::process::Command;

use akmon_bundle::generate_pkcs8;
use serde_json::Value;
use tempfile::tempdir;

#[allow(dead_code)]
mod common;
use common::*;

fn akmon() -> Command {
    Command::new(akmon_bin_path())
}

// FIXTURE_A: content present (full capture). Copied verbatim from
// crates/akmon-otel/src/lib.rs tests so the two stay consistent.
const FIXTURE_A: &str = r#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}},{"key":"gen_ai.system_instructions","value":{"stringValue":"[{\"type\":\"text\",\"content\":\"You are a helpful weather assistant.\"}]"}},{"key":"gen_ai.input.messages","value":{"stringValue":"[{\"role\":\"user\",\"parts\":[{\"type\":\"text\",\"content\":\"Weather in Paris?\"}]}]"}},{"key":"gen_ai.output.messages","value":{"stringValue":"[{\"role\":\"assistant\",\"parts\":[{\"type\":\"tool_call\",\"id\":\"call_x\",\"name\":\"get_weather\",\"arguments\":{\"location\":\"Paris\"}}],\"finish_reason\":\"tool_calls\"}]"}}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}},{"key":"gen_ai.tool.call.arguments","value":{"stringValue":"{\"location\":\"Paris\"}"}},{"key":"gen_ai.tool.call.result","value":{"stringValue":"rainy, 57F"}}]}]}]}]}"#;

// FIXTURE_B: metadata only (structural capture). Copied verbatim from
// crates/akmon-otel/src/lib.rs tests so the two stay consistent.
const FIXTURE_B: &str = r#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"agef-demo-agent"}}]},"scopeSpans":[{"scope":{"name":"opentelemetry.instrumentation.openai_v2"},"spans":[{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","parentSpanId":"","name":"chat gpt-4o","kind":3,"startTimeUnixNano":"1748000000000000000","endTimeUnixNano":"1748000001500000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.provider.name","value":{"stringValue":"openai"}},{"key":"gen_ai.request.model","value":{"stringValue":"gpt-4o"}},{"key":"gen_ai.response.model","value":{"stringValue":"gpt-4o-2024-08-06"}},{"key":"gen_ai.response.id","value":{"stringValue":"chatcmpl-Abc123"}},{"key":"gen_ai.conversation.id","value":{"stringValue":"conv-7f3a"}},{"key":"gen_ai.request.temperature","value":{"doubleValue":0.2}},{"key":"gen_ai.request.max_tokens","value":{"intValue":"512"}},{"key":"gen_ai.usage.input_tokens","value":{"intValue":"31"}},{"key":"gen_ai.usage.output_tokens","value":{"intValue":"19"}},{"key":"gen_ai.response.finish_reasons","value":{"arrayValue":{"values":[{"stringValue":"tool_calls"}]}}}]},{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"1a2b3c4d5e6f7081","parentSpanId":"00f067aa0ba902b7","name":"execute_tool get_weather","kind":1,"startTimeUnixNano":"1748000001500000000","endTimeUnixNano":"1748000001800000000","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"execute_tool"}},{"key":"gen_ai.tool.name","value":{"stringValue":"get_weather"}},{"key":"gen_ai.tool.call.id","value":{"stringValue":"call_x"}}]}]}]}]}"#;

// Legacy form: a single span carrying a `gen_ai.user.message` span event
// (semconv <= v1.36), which the importer must reject (F8).
const FIXTURE_LEGACY: &str = r#"{"resourceSpans":[{"scopeSpans":[{"spans":[{"traceId":"abcd","spanId":"1111","parentSpanId":"","name":"chat","kind":3,"startTimeUnixNano":"1","endTimeUnixNano":"2","attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},{"key":"gen_ai.system","value":{"stringValue":"openai"}}],"events":[{"name":"gen_ai.user.message","timeUnixNano":"1","attributes":[]}]}]}]}]}"#;

/// Writes `contents` to `<dir>/<name>` and returns the path display string.
fn write_fixture(dir: &std::path::Path, name: &str, contents: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write fixture");
    path.display().to_string()
}

/// FIXTURE_A imports with full capture: provider+tool counts and a session id.
#[test]
fn t_otel_import_full_capture() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path().join("journal");
    let trace = write_fixture(dir.path(), "trace_a.json", FIXTURE_A);

    let out = akmon()
        .args([
            "otel",
            "import",
            &trace,
            "--journal",
            &journal_dir.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run otel import");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("import json");
    assert_eq!(v.get("capture_level").and_then(Value::as_str), Some("full"));
    assert_eq!(v.get("provider_calls").and_then(Value::as_u64), Some(1));
    assert_eq!(v.get("tool_calls").and_then(Value::as_u64), Some(1));
    let sid = v
        .get("session_id")
        .and_then(Value::as_str)
        .expect("session_id");
    assert!(!sid.is_empty(), "session_id must be non-empty");
}

/// FIXTURE_B imports with structural capture: content was not captured by the source.
#[test]
fn t_otel_import_structural_capture() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path().join("journal");
    let trace = write_fixture(dir.path(), "trace_b.json", FIXTURE_B);

    let out = akmon()
        .args([
            "otel",
            "import",
            &trace,
            "--journal",
            &journal_dir.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run otel import");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("import json");
    assert_eq!(
        v.get("capture_level").and_then(Value::as_str),
        Some("structural")
    );
    assert!(
        v.get("turns_suppressed_no_content")
            .and_then(Value::as_u64)
            .expect("turns_suppressed_no_content")
            >= 1,
        "structural capture must suppress at least one turn"
    );
}

/// The full producer-agnostic loop, entirely through the akmon binary: a non-Akmon
/// OTel trace becomes a signed, independently-verifiable AGEF bundle.
#[test]
fn t_otel_import_then_sign_then_verify_roundtrip() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path().join("journal");
    let trace = write_fixture(dir.path(), "trace_a.json", FIXTURE_A);

    // a. Import the OTel trace into a fresh session; capture the session id.
    let import = akmon()
        .args([
            "otel",
            "import",
            &trace,
            "--journal",
            &journal_dir.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run otel import");
    assert_eq!(
        import.status.code(),
        Some(0),
        "import stderr={}",
        String::from_utf8_lossy(&import.stderr)
    );
    let import_json: Value = serde_json::from_slice(&import.stdout).expect("import json");
    let session_id = import_json
        .get("session_id")
        .and_then(Value::as_str)
        .expect("session_id")
        .to_owned();

    // b. Export the session as an AGEF bundle.
    let bundle = dir.path().join("s.akmon");
    let export = akmon()
        .args([
            "bundle",
            "export",
            &session_id,
            "--journal",
            &journal_dir.display().to_string(),
            "--output",
            &bundle.display().to_string(),
        ])
        .output()
        .expect("run bundle export");
    assert!(
        export.status.success(),
        "export stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );

    // c. Generate an Ed25519 PKCS#8 key.
    let key_path = dir.path().join("k.pk8");
    std::fs::write(&key_path, generate_pkcs8().expect("keygen")).expect("write key");

    // d. Sign the bundle; capture the published public key hex.
    let sign = akmon()
        .args([
            "bundle",
            "sign",
            &bundle.display().to_string(),
            "--key",
            &key_path.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle sign");
    assert!(
        sign.status.success(),
        "sign stderr={}",
        String::from_utf8_lossy(&sign.stderr)
    );
    let sign_json: Value = serde_json::from_slice(&sign.stdout).expect("sign json");
    let pubkey_hex = sign_json
        .get("public_key_hex")
        .and_then(Value::as_str)
        .expect("public_key_hex");
    let key_file = dir.path().join("k.pub.hex");
    std::fs::write(&key_file, pubkey_hex).expect("write pubkey hex");

    // e. Verify the signed bundle against the published public key.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--verify-key",
            &key_file.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "verify stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let verify_json: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(
        verify_json.get("passed").and_then(Value::as_bool),
        Some(true)
    );
    let sigs = verify_json
        .get("signatures")
        .and_then(Value::as_array)
        .expect("signatures");
    assert_eq!(
        sigs[0].get("outcome").and_then(Value::as_str),
        Some("verified")
    );
}

/// A legacy (semconv <= v1.36) message-event trace is a usage error (exit 2).
#[test]
fn t_otel_import_rejects_legacy() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path().join("journal");
    let trace = write_fixture(dir.path(), "trace_legacy.json", FIXTURE_LEGACY);

    let out = akmon()
        .args([
            "otel",
            "import",
            &trace,
            "--journal",
            &journal_dir.display().to_string(),
        ])
        .output()
        .expect("run otel import");
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}
