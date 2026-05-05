use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_core::{AgentConfig, Permission, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::{AttemptRecord, AttemptStatus, EventKind, SessionGraph};
use akmon_models::{
    CompletionStream, LlmProvider, ModelError, ModelToolCall, StopReason, StreamEvent,
};
use akmon_query::AgentSession;
use akmon_tools::{Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use futures_util::stream;
use serde::Deserialize;
use serde_json::Value;
use tempfile::tempdir;
use time::{Duration, OffsetDateTime};
use tokio::sync::mpsc;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::*;

fn run_inspect(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    run_inspect_with(journal_dir, session_id, &[])
}

fn run_inspect_verbose(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    run_inspect_with(journal_dir, session_id, &["--verbose"])
}

fn run_inspect_with(
    journal_dir: &Path,
    session_id: Uuid,
    extra_args: &[&str],
) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "inspect",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra_args);
    cmd.output().expect("run inspect")
}

fn run_redact_with(
    journal_dir: &Path,
    session_id: Uuid,
    output_path: &Path,
    objects: &[&str],
    reason: &str,
    extra: &[&str],
) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "redact",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
        "--output",
        &output_path.display().to_string(),
    ]);
    for object in objects {
        cmd.args(["--object", object]);
    }
    cmd.args(["--reason", reason]);
    cmd.args(extra);
    cmd.output().expect("run redact")
}

fn run_bundle_import_with(
    bundle: &Path,
    journal_dir: &Path,
    extra: &[&str],
) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "bundle",
        "import",
        bundle.to_str().expect("utf8 path"),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra);
    cmd.output().expect("run bundle import")
}

fn parse_human_summary_event_kinds(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            if line.starts_with('[') {
                let mut parts = line.split_whitespace();
                let _seq = parts.next()?;
                let kind = parts.next()?;
                Some(kind.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn parse_human_summary_hash_prefixes(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.split("hash=").nth(1))
        .map(|rest| rest.trim().to_owned())
        .collect()
}

fn inspect_kind_name(kind: &InspectEventKind) -> &'static str {
    match kind {
        InspectEventKind::SessionStart { .. } => "SessionStart",
        InspectEventKind::UserTurn { .. } => "UserTurn",
        InspectEventKind::ProviderCall { .. } => "ProviderCall",
        InspectEventKind::ToolCall { .. } => "ToolCall",
        InspectEventKind::RetrievalCall { .. } => "RetrievalCall",
        InspectEventKind::PermissionGate { .. } => "PermissionGate",
        InspectEventKind::AssistantTurn { .. } => "AssistantTurn",
        InspectEventKind::SessionEnd { .. } => "SessionEnd",
    }
}

fn contains_truncated_hash(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 11 <= bytes.len() {
        let candidate = &text[i..i + 11];
        let chars: Vec<char> = candidate.chars().collect();
        if chars.len() == 11
            && chars[8] == '.'
            && chars[9] == '.'
            && chars[10] == '.'
            && chars[..8].iter().all(|c| c.is_ascii_hexdigit())
        {
            return true;
        }
        i += 1;
    }
    false
}

fn contains_full_hash(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 64 <= bytes.len() {
        let candidate = &text[i..i + 64];
        if candidate.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }
        i += 1;
    }
    false
}

fn contains_iso_timestamp(text: &str) -> bool {
    text.contains("T") && text.contains("Z") && text.contains("emitted_at:")
}

fn looks_like_hex_pairs(text: &str) -> bool {
    text.contains("fe ed fa ce") || text.contains("ff 00 01 02")
}

fn looks_like_base64_preview(text: &str) -> bool {
    text.contains("/wABAg==") || text.contains("/u36zg==")
}

#[derive(Debug, Deserialize)]
struct InspectReportV1 {
    akmon_version: String,
    agef_version: String,
    session_id: String,
    journal_path: String,
    events: Vec<InspectEvent>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct InspectEvent {
    sequence: u64,
    event_hash: String,
    parent_hashes: Vec<String>,
    emitted_at: String,
    kind: InspectEventKind,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum InspectEventKind {
    SessionStart {
        cwd_hash: String,
        config_hash: String,
    },
    UserTurn {
        prompt_hash: String,
        prompt_text: Option<String>,
        prompt_size: Option<u64>,
    },
    ProviderCall {
        provider_id: String,
        attempts: Vec<InspectAttempt>,
        stream_hash: Option<String>,
        stream_text: Option<String>,
        stream_size: Option<u64>,
    },
    ToolCall {
        tool_id: String,
        input_hash: String,
        output_hash: String,
        side_effects_hash: Option<String>,
        input_text: Option<String>,
        input_size: Option<u64>,
        output_text: Option<String>,
        output_size: Option<u64>,
    },
    RetrievalCall {
        index_id: String,
        query_hash: String,
        results_hash: String,
    },
    PermissionGate {
        policy_id: String,
        decision: String,
        context_hash: String,
    },
    AssistantTurn {
        message_hash: String,
        tool_calls_hash: Option<String>,
        message_text: Option<String>,
        message_size: Option<u64>,
        tool_calls_text: Option<String>,
        tool_calls_size: Option<u64>,
    },
    SessionEnd {
        summary_hash: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct InspectAttempt {
    attempt_number: u32,
    status: String,
    started_at: String,
    ended_at: String,
    request_hash: String,
    response_hash: Option<String>,
    stream_hash: Option<String>,
    error_message: Option<String>,
    request_text: Option<String>,
    request_size: Option<u64>,
    response_text: Option<String>,
    response_size: Option<u64>,
    stream_text: Option<String>,
    stream_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct InspectError {
    akmon_version: String,
    category: String,
    error: String,
}

#[test]
fn t_inspect_displays_clean_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&format!("session: {sid}")));
    assert!(stdout.contains("events: 3"));
    assert!(stdout.contains("[0] SessionStart"));
    assert!(stdout.contains("[1] UserTurn"));
    assert!(stdout.contains("[2] SessionEnd"));
    assert!(
        contains_truncated_hash(&stdout),
        "expected truncated hash (8 hex + ...), got:\n{stdout}"
    );
}

#[test]
fn t_inspect_json_output_for_clean_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: InspectReportV1 = serde_json::from_str(&stdout).expect("parse inspect report");
    assert!(!parsed.akmon_version.is_empty());
    assert_eq!(parsed.agef_version, "0.1.1");
    assert_eq!(parsed.session_id, sid.to_string());
    assert!(!parsed.journal_path.is_empty());
    assert_eq!(parsed.events.len(), 3);
    assert!(matches!(
        parsed.events[0].kind,
        InspectEventKind::SessionStart { .. }
    ));
    assert!(matches!(
        parsed.events[1].kind,
        InspectEventKind::UserTurn { .. }
    ));
    assert!(matches!(
        parsed.events[2].kind,
        InspectEventKind::SessionEnd { .. }
    ));
}

#[test]
fn t_inspect_verbose_with_resolve_human() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--verbose", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(contains_full_hash(&stdout));
    assert!(stdout.contains("parent:"));
    assert!(stdout.contains("emitted_at:"));
    assert!(stdout.contains("| hello"));
}

#[test]
fn t_inspect_verbose_with_resolve_binary_hex() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFE, 0xED, 0xFA, 0xCE])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(
        tmp.path(),
        sid,
        &["--verbose", "--resolve", "--binary", "hex"],
    );
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(contains_full_hash(&stdout));
    assert!(looks_like_hex_pairs(&stdout));
    assert!(stdout.contains("  | "));
}

#[test]
fn t_inspect_format_json_with_verbose_byte_identical() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let base = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
    let with_verbose = run_inspect_with(tmp.path(), sid, &["--format", "json", "--verbose"]);
    assert_eq!(base.status.code(), Some(0));
    assert_eq!(with_verbose.status.code(), Some(0));
    assert_eq!(base.stdout, with_verbose.stdout);
}

#[test]
fn t_inspect_handles_empty_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(put_bytes(store.as_ref(), b"done")),
            })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("events: 2"));
    assert!(stdout.contains("SessionStart"));
    assert!(stdout.contains("SessionEnd"));
}

#[tokio::test]
async fn t_inspect_handles_single_turn_no_tools() {
    struct TextOnlyMockProvider;
    #[async_trait]
    impl LlmProvider for TextOnlyMockProvider {
        fn name(&self) -> &str {
            "text-only-mock"
        }
        fn context_window_tokens(&self) -> usize {
            200_000
        }
        fn completion_model_id(&self) -> &str {
            "text-only-model"
        }
        async fn complete(
            &self,
            _messages: &[akmon_models::Message],
            _config: &akmon_models::CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::TextDelta {
                    text: "single turn".into(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                }),
            ])))
        }
    }

    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    let cfg = AgentConfig {
        max_iterations: 8,
        confirmation_timeout_secs: 30,
        session_id: sid,
        auto_commit: false,
        max_completion_tokens: None,
        subagent_style: false,
        max_budget_usd: None,
        fallback_model: None,
        model_estimates: Vec::new(),
    };
    let mut session = AgentSession::new(
        cfg,
        Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        })),
        Arc::new(TextOnlyMockProvider),
        vec![],
        Arc::new(Sandbox::new(tmp.path())),
        None,
        false,
        journal,
    )
    .expect("session");
    let (tx, _rx) = mpsc::channel(32);
    let mut no_policy = None;
    session
        .run("just text".into(), tx, &mut no_policy, &mut None, None)
        .await
        .expect("run");
    session.end(None).expect("end");
    drop(session);

    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("events: 5"));
    assert!(stdout.contains("SessionStart"));
    assert!(stdout.contains("UserTurn"));
    assert!(stdout.contains("ProviderCall"));
    assert!(stdout.contains("AssistantTurn"));
    assert!(stdout.contains("SessionEnd"));
    assert!(!stdout.contains("ToolCall"));
    assert!(!stdout.contains("PermissionGate"));
}

#[test]
fn t_inspect_human_and_json_describe_same_events() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::UserTurn {
                prompt_hash: put_bytes(store.as_ref(), b"user prompt"),
            })
            .expect("append user");
        graph
            .append(EventKind::PermissionGate {
                policy_id: "tool:auto".to_owned(),
                decision: "allowed".to_owned(),
                context_hash: put_bytes(store.as_ref(), br#"{"tool":"read_file"}"#),
            })
            .expect("append gate");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: None,
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);

    let human = run_inspect(tmp.path(), sid);
    let json = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(human.status.code(), Some(0));
    assert_eq!(json.status.code(), Some(0));

    let human_stdout = String::from_utf8_lossy(&human.stdout);
    let human_kinds = parse_human_summary_event_kinds(&human_stdout);
    let human_hash_prefixes = parse_human_summary_hash_prefixes(&human_stdout);

    let parsed: InspectReportV1 = serde_json::from_slice(&json.stdout).expect("parse json");
    let json_kinds: Vec<String> = parsed
        .events
        .iter()
        .map(|e| inspect_kind_name(&e.kind).to_owned())
        .collect();
    assert_eq!(human_kinds.len(), parsed.events.len());
    assert_eq!(human_kinds, json_kinds);
    for (idx, event) in parsed.events.iter().enumerate() {
        assert!(
            human_hash_prefixes[idx].starts_with(&event.event_hash[..8]),
            "human hash prefix mismatch at {idx}"
        );
    }
}

#[test]
fn t_inspect_displays_permission_gate() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::PermissionGate {
                policy_id: "tool:auto".to_owned(),
                decision: "allowed".to_owned(),
                context_hash: put_bytes(store.as_ref(), br#"{"tool":"read_file"}"#),
            })
            .expect("append gate");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out_summary = run_inspect(tmp.path(), sid);
    assert_eq!(out_summary.status.code(), Some(0));
    let summary = String::from_utf8_lossy(&out_summary.stdout);
    assert!(summary.contains("PermissionGate"));
    assert!(summary.contains("policy: tool:auto"));
    assert!(summary.contains("decision: allowed"));
    assert!(summary.contains("context_hash:"));
    let out_verbose = run_inspect_verbose(tmp.path(), sid);
    assert_eq!(out_verbose.status.code(), Some(0));
    let verbose = String::from_utf8_lossy(&out_verbose.stdout);
    assert!(contains_full_hash(&verbose));
}

#[test]
fn t_inspect_displays_retrieval_call_in_synthetic_journal() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::RetrievalCall {
                index_id: "idx-main".to_owned(),
                query_hash: put_bytes(store.as_ref(), b"query"),
                results_hash: put_bytes(store.as_ref(), b"results"),
            })
            .expect("append retrieval");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);

    let summary = run_inspect(tmp.path(), sid);
    assert_eq!(summary.status.code(), Some(0));
    let summary_out = String::from_utf8_lossy(&summary.stdout);
    assert!(summary_out.contains("RetrievalCall"));
    assert!(summary_out.contains("index_id: idx-main"));
    assert!(summary_out.contains("query_hash:"));
    assert!(summary_out.contains("results_hash:"));

    let verbose = run_inspect_verbose(tmp.path(), sid);
    assert_eq!(verbose.status.code(), Some(0));
    let verbose_out = String::from_utf8_lossy(&verbose.stdout);
    assert!(contains_full_hash(&verbose_out));

    let json = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(json.status.code(), Some(0));
    let parsed: InspectReportV1 = serde_json::from_slice(&json.stdout).expect("json parse");
    assert!(parsed.events.iter().any(|ev| {
        matches!(
            ev.kind,
            InspectEventKind::RetrievalCall {
                index_id: _,
                query_hash: _,
                results_hash: _
            }
        )
    }));
}

#[test]
fn t_inspect_output_stability() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);

    let human_a = run_inspect(tmp.path(), sid);
    let human_b = run_inspect(tmp.path(), sid);
    assert_eq!(human_a.status.code(), Some(0));
    assert_eq!(human_b.status.code(), Some(0));
    assert_eq!(human_a.stdout, human_b.stdout);

    let json_a = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
    let json_b = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(json_a.status.code(), Some(0));
    assert_eq!(json_b.status.code(), Some(0));
    assert_eq!(json_a.stdout, json_b.stdout);

    let verbose_hex_a = run_inspect_with(
        tmp.path(),
        sid,
        &["--verbose", "--resolve", "--binary", "hex"],
    );
    let verbose_hex_b = run_inspect_with(
        tmp.path(),
        sid,
        &["--verbose", "--resolve", "--binary", "hex"],
    );
    assert_eq!(verbose_hex_a.status.code(), Some(0));
    assert_eq!(verbose_hex_b.status.code(), Some(0));
    assert_eq!(verbose_hex_a.stdout, verbose_hex_b.stdout);
}

#[test]
fn t_inspect_resolve_human_shows_text() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("prompt_hash:"));
    assert!(stdout.contains("| hello"));
}

#[test]
fn t_inspect_resolve_human_shows_binary_metadata() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFF, 0x00, 0x01, 0x02])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("<binary, 4 bytes"));
}

#[test]
fn t_inspect_resolve_binary_hex_mode() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFE, 0xED, 0xFA, 0xCE])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve", "--binary", "hex"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(looks_like_hex_pairs(&stdout));
}

#[test]
fn t_inspect_resolve_binary_base64_mode() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFF, 0x00, 0x01, 0x02])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve", "--binary", "base64"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(looks_like_base64_preview(&stdout));
}

#[test]
fn t_inspect_resolve_binary_meta_default() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFF, 0x00, 0x01, 0x02])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("<binary, 4 bytes"));
    assert!(!looks_like_hex_pairs(&stdout));
    assert!(!looks_like_base64_preview(&stdout));
}

#[test]
fn t_inspect_resolve_binary_truncation() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        let large: Vec<u8> = vec![0xFF; 120];
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), large.as_slice())),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve", "--binary", "hex"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("truncated"));
    assert!(stdout.contains("more bytes"));
}

#[test]
fn t_inspect_resolve_binary_short_content_no_truncation() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFE, 0xED, 0xFA, 0xCE])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve", "--binary", "hex"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(looks_like_hex_pairs(&stdout));
    assert!(!stdout.contains("more bytes"));
}

#[test]
fn t_inspect_resolve_truncates_long_text() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        let long = "l1\nl2\nl3\nl4\nl5\nl6\nl7";
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::UserTurn {
                prompt_hash: put_bytes(store.as_ref(), long.as_bytes()),
            })
            .expect("append user");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("| l1"));
    assert!(stdout.contains("| l5"));
    assert!(stdout.contains("more lines"));
}

#[test]
fn t_inspect_resolve_handles_missing_object() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let store = Arc::new(
        akmon_journal::RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("open"),
    );
    store
        .remove_object_for_testing(&fixture.prompt_hash)
        .expect("remove prompt");
    drop(store);
    let out = run_inspect_with(tmp.path(), sid, &["--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("<unresolved>"));
}

#[test]
fn t_inspect_json_field_stability() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let value: Value = serde_json::from_slice(&out.stdout).expect("parse generic json");
    assert!(value.get("akmon_version").is_some());
    assert!(value.get("agef_version").is_some());
    assert!(value.get("session_id").is_some());
    assert!(value.get("journal_path").is_some());
    assert!(value.get("events").is_some());
    let events = value
        .get("events")
        .and_then(Value::as_array)
        .expect("events array");
    assert!(!events.is_empty());
    let first = &events[0];
    assert!(first.get("sequence").is_some());
    assert!(first.get("event_hash").is_some());
    assert!(first.get("parent_hashes").is_some());
    assert!(first.get("emitted_at").is_some());
    assert!(first.get("kind").is_some());
}

#[test]
fn t_inspect_json_provider_call_includes_full_attempts() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::ProviderCall {
                provider_id: "json-provider".to_owned(),
                attempts: vec![
                    AttemptRecord {
                        attempt_number: 1,
                        started_at: OffsetDateTime::UNIX_EPOCH,
                        ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
                        status: AttemptStatus::RateLimited,
                        request_hash: put_bytes(store.as_ref(), b"req-a"),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some("429".to_owned()),
                    },
                    AttemptRecord {
                        attempt_number: 2,
                        started_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(2),
                        ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(3),
                        status: AttemptStatus::Success,
                        request_hash: put_bytes(store.as_ref(), b"req-b"),
                        response_hash: Some(put_bytes(store.as_ref(), b"resp-b")),
                        stream_hash: Some(put_bytes(store.as_ref(), b"stream-b")),
                        error_message: None,
                    },
                ],
                stream_hash: Some(put_bytes(store.as_ref(), b"outer-stream")),
            })
            .expect("append provider call");
        graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
            })
            .expect("append end");
    }
    drop(journal);

    let out = run_inspect_with(tmp.path(), sid, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let parsed: InspectReportV1 = serde_json::from_slice(&out.stdout).expect("parse inspect json");
    let provider = parsed
        .events
        .iter()
        .find_map(|event| match &event.kind {
            InspectEventKind::ProviderCall {
                provider_id,
                attempts,
                stream_hash,
                ..
            } => Some((provider_id, attempts, stream_hash)),
            _ => None,
        })
        .expect("provider call event");
    assert_eq!(provider.0, "json-provider");
    assert_eq!(provider.1.len(), 2);
    assert_eq!(provider.1[0].attempt_number, 1);
    assert_eq!(provider.1[1].attempt_number, 2);
    assert!(!provider.1[0].status.is_empty());
    assert!(!provider.1[0].started_at.is_empty());
    assert!(!provider.1[0].ended_at.is_empty());
    assert!(!provider.1[0].request_hash.is_empty());
    assert!(provider.1[0].response_hash.is_none());
    assert!(provider.1[1].response_hash.is_some());
    assert!(provider.1[1].stream_hash.is_some());
    assert!(provider.2.is_some());
    assert!(
        provider.1[0].request_size.is_some() || provider.1[0].request_text.is_some(),
        "expected resolved request payload metadata/text"
    );
}

#[test]
fn t_inspect_displays_provider_call_attempt_summary() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::ProviderCall {
                provider_id: "integration-provider".to_owned(),
                attempts: vec![
                    AttemptRecord {
                        attempt_number: 1,
                        started_at: OffsetDateTime::UNIX_EPOCH,
                        ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
                        status: AttemptStatus::RateLimited,
                        request_hash: put_bytes(store.as_ref(), b"req-1"),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some("429".to_owned()),
                    },
                    AttemptRecord {
                        attempt_number: 2,
                        started_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(2),
                        ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(3),
                        status: AttemptStatus::Success,
                        request_hash: put_bytes(store.as_ref(), b"req-2"),
                        response_hash: Some(put_bytes(store.as_ref(), b"resp-2")),
                        stream_hash: None,
                        error_message: None,
                    },
                ],
                stream_hash: Some(put_bytes(store.as_ref(), b"stream-2")),
            })
            .expect("append provider call");
        graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
            })
            .expect("append end");
    }
    drop(journal);

    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ProviderCall"));
    assert!(stdout.contains("attempts: 2 attempts: 1 RateLimited, 1 Success"));
}

#[test]
fn t_inspect_verbose_shows_full_hashes() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_verbose(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        contains_full_hash(&stdout),
        "expected at least one full hash, got:\n{stdout}"
    );
    assert!(stdout.contains("parent:"));
}

#[test]
fn t_inspect_verbose_shows_attempt_records() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::ProviderCall {
                provider_id: "integration-provider".to_owned(),
                attempts: vec![
                    AttemptRecord {
                        attempt_number: 1,
                        started_at: OffsetDateTime::UNIX_EPOCH,
                        ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
                        status: AttemptStatus::RateLimited,
                        request_hash: put_bytes(store.as_ref(), b"req-1"),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some("429".to_owned()),
                    },
                    AttemptRecord {
                        attempt_number: 2,
                        started_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(2),
                        ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(3),
                        status: AttemptStatus::Success,
                        request_hash: put_bytes(store.as_ref(), b"req-2"),
                        response_hash: Some(put_bytes(store.as_ref(), b"resp-2")),
                        stream_hash: Some(put_bytes(store.as_ref(), b"stream-2")),
                        error_message: None,
                    },
                ],
                stream_hash: Some(put_bytes(store.as_ref(), b"stream-outer")),
            })
            .expect("append provider");
        graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
            })
            .expect("append end");
    }
    drop(journal);

    let out = run_inspect_verbose(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("attempts:"));
    assert!(stdout.contains("[1] RateLimited"));
    assert!(stdout.contains("[2] Success"));
    assert!(stdout.contains("request_hash:"));
    assert!(stdout.contains("response_hash:"));
}

#[test]
fn t_inspect_verbose_shows_emitted_at() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_verbose(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        contains_iso_timestamp(&stdout),
        "expected emitted_at ISO timestamp, got:\n{stdout}"
    );
}

#[test]
fn t_inspect_non_verbose_unchanged() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[0] SessionStart  hash="));
    assert!(contains_truncated_hash(&stdout));
    assert!(!stdout.contains("emitted_at:"));
    assert!(!stdout.contains("parent: "));
}

#[test]
fn t_inspect_resolve_does_not_affect_non_resolve_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let baseline = run_inspect(tmp.path(), sid);
    let with_resolve = run_inspect_with(tmp.path(), sid, &["--resolve"]);
    assert_eq!(baseline.status.code(), Some(0));
    assert_eq!(with_resolve.status.code(), Some(0));
    let baseline_stdout = String::from_utf8_lossy(&baseline.stdout);
    assert!(!baseline_stdout.contains("| hello"));
    assert!(baseline_stdout.contains("prompt_hash:"));
}

#[test]
fn t_inspect_fails_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let missing = Uuid::new_v4();
    let out = run_inspect(tmp.path(), missing);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open journal"));
}

#[test]
fn t_inspect_json_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let missing = Uuid::new_v4();
    let out = run_inspect_with(tmp.path(), missing, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(3));
    let parsed: InspectError = serde_json::from_slice(&out.stdout).expect("parse inspect error");
    assert!(!parsed.akmon_version.is_empty());
    assert_eq!(parsed.category, "session_not_found");
    assert!(!parsed.error.is_empty());
}

#[test]
fn t_inspect_fails_for_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let missing_dir = tmp.path().join("does-not-exist");
    let out = run_inspect(missing_dir.as_path(), sid);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open journal"));
}

#[test]
fn t_inspect_json_for_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let missing_dir = tmp.path().join("does-not-exist");
    let out = run_inspect_with(missing_dir.as_path(), sid, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(3));
    let parsed: InspectError = serde_json::from_slice(&out.stdout).expect("parse inspect error");
    assert!(!parsed.akmon_version.is_empty());
    assert_eq!(parsed.category, "journal_not_found");
    assert!(!parsed.error.is_empty());
}

#[test]
fn t_inspect_json_with_resolve_outputs_text() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let parsed: InspectReportV1 = serde_json::from_slice(&out.stdout).expect("json parse");
    let user = parsed
        .events
        .iter()
        .find_map(|event| match &event.kind {
            InspectEventKind::UserTurn {
                prompt_text,
                prompt_size,
                ..
            } => Some((prompt_text, prompt_size)),
            _ => None,
        })
        .expect("user turn");
    assert_eq!(user.0.as_deref(), Some("hello"));
    assert!(user.1.is_some());
}

#[test]
fn t_inspect_json_resolve_binary_no_text_field() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"text-response"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), &[0xFE, 0xED, 0xFA, 0xCE])),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .expect("append end");
    }
    drop(journal);
    let out = run_inspect_with(tmp.path(), sid, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let parsed: InspectReportV1 = serde_json::from_slice(&out.stdout).expect("json parse");
    let assistant = parsed
        .events
        .iter()
        .find_map(|event| match &event.kind {
            InspectEventKind::AssistantTurn {
                tool_calls_text,
                tool_calls_size,
                ..
            } => Some((tool_calls_text, tool_calls_size)),
            _ => None,
        })
        .expect("assistant turn");
    assert!(assistant.0.is_none());
    assert_eq!(assistant.1, &Some(4));
}

#[test]
fn t_inspect_json_resolve_unaffected_by_binary_mode() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let base = run_inspect_with(tmp.path(), sid, &["--format", "json", "--resolve"]);
    let hex = run_inspect_with(
        tmp.path(),
        sid,
        &["--format", "json", "--resolve", "--binary", "hex"],
    );
    let b64 = run_inspect_with(
        tmp.path(),
        sid,
        &["--format", "json", "--resolve", "--binary", "base64"],
    );
    assert_eq!(base.status.code(), Some(0));
    assert_eq!(hex.status.code(), Some(0));
    assert_eq!(b64.status.code(), Some(0));
    let v_base: Value = serde_json::from_slice(&base.stdout).expect("base json");
    let v_hex: Value = serde_json::from_slice(&hex.stdout).expect("hex json");
    let v_b64: Value = serde_json::from_slice(&b64.stdout).expect("b64 json");
    assert_eq!(v_base, v_hex);
    assert_eq!(v_base, v_b64);
}

#[test]
fn t_inspect_displays_sentinel_in_human_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let bundle = tmp.path().join("redacted.akmon");
    let redaction_reason = "PII removal";
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        redaction_reason,
        &[],
    );
    assert_eq!(redact.status.code(), Some(0));

    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir");
    let renamed = Uuid::new_v4();
    let import = run_bundle_import_with(
        bundle.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(import.status.code(), Some(0));

    let out = run_inspect_with(dst.as_path(), renamed, &["--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[REDACTED:"));
    assert!(stdout.contains(redaction_reason));
    assert!(stdout.contains("original size:"));
    assert!(stdout.contains("original hash:"));
    assert!(stdout.contains("redacted at:"));
}

#[test]
fn t_inspect_displays_sentinel_in_json_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let bundle = tmp.path().join("redacted.akmon");
    let redaction_reason = "Trade secret";
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        redaction_reason,
        &["--format", "json"],
    );
    assert_eq!(redact.status.code(), Some(0));

    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir");
    let renamed = Uuid::new_v4();
    let import = run_bundle_import_with(
        bundle.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(import.status.code(), Some(0));

    let out = run_inspect_with(dst.as_path(), renamed, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let value: Value = serde_json::from_slice(&out.stdout).expect("json");
    let events = value
        .get("events")
        .and_then(Value::as_array)
        .expect("events array");
    let user_kind = events
        .iter()
        .filter_map(|event| event.get("kind").and_then(Value::as_object))
        .find(|kind| kind.get("type").and_then(Value::as_str) == Some("user_turn"))
        .expect("user turn kind");
    assert_eq!(
        user_kind.get("prompt_redacted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        user_kind
            .get("prompt_redaction_reason")
            .and_then(Value::as_str),
        Some(redaction_reason)
    );
    let original_hash = fixture.prompt_hash.to_hex();
    assert_eq!(
        user_kind
            .get("prompt_original_hash")
            .and_then(Value::as_str),
        Some(original_hash.as_str())
    );
    assert!(user_kind.get("prompt_original_size").is_some());
    assert!(user_kind.get("prompt_redacted_at").is_some());
    assert!(user_kind.get("prompt_size").is_some());
}

#[test]
fn t_inspect_non_sentinel_objects_unchanged() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let bundle = tmp.path().join("redacted.akmon");
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "PII",
        &[],
    );
    assert_eq!(redact.status.code(), Some(0));
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir");
    let renamed = Uuid::new_v4();
    let import = run_bundle_import_with(
        bundle.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(import.status.code(), Some(0));
    let out = run_inspect_with(dst.as_path(), renamed, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let value: Value = serde_json::from_slice(&out.stdout).expect("json");
    let events = value
        .get("events")
        .and_then(Value::as_array)
        .expect("events array");
    let summary_kind = events
        .iter()
        .filter_map(|event| event.get("kind").and_then(Value::as_object))
        .find(|kind| kind.get("type").and_then(Value::as_str) == Some("session_end"))
        .expect("session_end kind");
    assert!(summary_kind.get("summary_redacted").is_none());
    assert!(summary_kind.get("summary_redaction_reason").is_none());
}

#[test]
fn t_inspect_verbose_with_sentinel_shows_both_timestamps() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let bundle = tmp.path().join("redacted.akmon");
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "PII",
        &[],
    );
    assert_eq!(redact.status.code(), Some(0));
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir");
    let renamed = Uuid::new_v4();
    let import = run_bundle_import_with(
        bundle.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(import.status.code(), Some(0));
    let out = run_inspect_with(dst.as_path(), renamed, &["--verbose", "--resolve"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("emitted_at:"));
    assert!(stdout.contains("redacted at:"));
}

#[test]
fn t_inspect_binary_hex_with_sentinel_still_shows_redacted_ui() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let bundle = tmp.path().join("redacted.akmon");
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "PII",
        &[],
    );
    assert_eq!(redact.status.code(), Some(0));
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir");
    let renamed = Uuid::new_v4();
    let import = run_bundle_import_with(
        bundle.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(import.status.code(), Some(0));
    let out = run_inspect_with(dst.as_path(), renamed, &["--resolve", "--binary", "hex"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[REDACTED:"));
    assert!(!stdout.contains("a4 6b 61 6b 6d 6f 6e"));
}

#[test]
fn t_inspect_binary_hex_without_resolve_still_fails() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--binary", "hex"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("requires --resolve"));
}

#[test]
fn t_inspect_handles_session_with_all_event_kinds() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal");
    {
        let mut graph = journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let store = &journal.store;
        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .expect("append start");
        graph
            .append(EventKind::UserTurn {
                prompt_hash: put_bytes(store.as_ref(), b"user prompt"),
            })
            .expect("append user");
        graph
            .append(EventKind::ProviderCall {
                provider_id: "provider-x".to_owned(),
                attempts: vec![AttemptRecord {
                    attempt_number: 1,
                    started_at: OffsetDateTime::UNIX_EPOCH,
                    ended_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1),
                    status: AttemptStatus::Success,
                    request_hash: put_bytes(store.as_ref(), b"req"),
                    response_hash: Some(put_bytes(store.as_ref(), b"resp")),
                    stream_hash: Some(put_bytes(store.as_ref(), b"attempt-stream")),
                    error_message: None,
                }],
                stream_hash: Some(put_bytes(store.as_ref(), b"stream")),
            })
            .expect("append provider");
        graph
            .append(EventKind::ToolCall {
                tool_id: "read_file".to_owned(),
                input_hash: put_bytes(store.as_ref(), br#"{"path":"Cargo.toml"}"#),
                output_hash: put_bytes(store.as_ref(), b"ok"),
                side_effects_hash: None,
            })
            .expect("append tool");
        graph
            .append(EventKind::PermissionGate {
                policy_id: "default".to_owned(),
                decision: "allowed".to_owned(),
                context_hash: put_bytes(store.as_ref(), br#"{"tool":"read_file"}"#),
            })
            .expect("append gate");
        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"assistant response"),
                tool_calls_hash: Some(put_bytes(store.as_ref(), b"tool-calls")),
            })
            .expect("append assistant");
        graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(put_bytes(store.as_ref(), b"done")),
            })
            .expect("append end");
    }
    drop(journal);

    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);

    let order = [
        "SessionStart",
        "UserTurn",
        "ProviderCall",
        "ToolCall",
        "PermissionGate",
        "AssistantTurn",
        "SessionEnd",
    ];
    let mut prev = 0usize;
    for kind in order {
        let pos = stdout.find(kind).expect("kind appears");
        assert!(pos >= prev, "event kind {kind} out of order");
        prev = pos;
    }
}

const TOOL_NAME: &str = "inspect_search_like";
const TOOL_OUTPUT: &str = "INSPECT_TOOL_OUTPUT";

fn search_tool_perms() -> &'static [Permission] {
    use std::sync::OnceLock;
    static CELL: OnceLock<Vec<Permission>> = OnceLock::new();
    CELL.get_or_init(|| {
        vec![Permission::ReadFile {
            path: std::path::PathBuf::from("."),
        }]
    })
    .as_slice()
}

struct IntegrationSearchTool;

#[async_trait]
impl Tool for IntegrationSearchTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }
    fn description(&self) -> &str {
        "integration search mock"
    }
    fn required_permissions(&self) -> &[Permission] {
        search_tool_perms()
    }
    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::Success {
            content: TOOL_OUTPUT.into(),
        }
    }
}

struct OneTurnMockProvider {
    sequences: Vec<Vec<Result<StreamEvent, ModelError>>>,
    call: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for OneTurnMockProvider {
    fn name(&self) -> &str {
        "integration-mock"
    }
    fn context_window_tokens(&self) -> usize {
        200_000
    }
    fn completion_model_id(&self) -> &str {
        "integration-mock-model"
    }
    async fn complete(
        &self,
        _messages: &[akmon_models::Message],
        _config: &akmon_models::CompletionConfig,
    ) -> Result<CompletionStream, ModelError> {
        use std::sync::atomic::Ordering;
        let i = self.call.fetch_add(1, Ordering::SeqCst);
        let events = self.sequences.get(i).cloned().unwrap_or_default();
        Ok(Box::pin(stream::iter(events)))
    }
}

fn one_turn_sequences() -> Vec<Vec<Result<StreamEvent, ModelError>>> {
    vec![
        vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            tool_calls: vec![ModelToolCall {
                id: "call-1".into(),
                name: TOOL_NAME.into(),
                arguments: serde_json::json!({"query": "widgets"}),
            }],
        })],
        vec![
            Ok(StreamEvent::TextDelta {
                text: "final response".into(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ],
    ]
}

#[tokio::test]
async fn t_inspect_full_session_via_real_agent_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let journal = open_journal_handle(tmp.path(), sid).expect("journal create");
    let cfg = AgentConfig {
        max_iterations: 8,
        confirmation_timeout_secs: 30,
        session_id: sid,
        auto_commit: false,
        max_completion_tokens: None,
        subagent_style: false,
        max_budget_usd: None,
        fallback_model: None,
        model_estimates: Vec::new(),
    };

    let mut session = AgentSession::new(
        cfg,
        Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        })),
        Arc::new(OneTurnMockProvider {
            sequences: one_turn_sequences(),
            call: std::sync::atomic::AtomicUsize::new(0),
        }),
        vec![Box::new(IntegrationSearchTool)],
        Arc::new(Sandbox::new(tmp.path())),
        None,
        false,
        journal,
    )
    .expect("session new");

    let (tx, _rx) = mpsc::channel(64);
    let mut no_policy = None;
    session
        .run("one turn".into(), tx, &mut no_policy, &mut None, None)
        .await
        .expect("run");
    session.end(None).expect("session end");
    drop(session);

    let out = run_inspect(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for kind in [
        "SessionStart",
        "UserTurn",
        "ProviderCall",
        "ToolCall",
        "PermissionGate",
        "AssistantTurn",
        "SessionEnd",
    ] {
        assert!(stdout.contains(kind), "missing {kind} in output");
    }
}
