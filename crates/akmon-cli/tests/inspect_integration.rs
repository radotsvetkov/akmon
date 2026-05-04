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
    let bin = std::env::var("CARGO_BIN_EXE_akmon").expect("CARGO_BIN_EXE_akmon");
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
    },
    ProviderCall {
        provider_id: String,
        attempts: Vec<InspectAttempt>,
        stream_hash: Option<String>,
    },
    ToolCall {
        tool_id: String,
        input_hash: String,
        output_hash: String,
        side_effects_hash: Option<String>,
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
fn t_inspect_json_field_stability() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
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

    let out = run_inspect_with(tmp.path(), sid, &["--format", "json"]);
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
fn t_inspect_json_with_resolve_layer4() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_inspect_with(tmp.path(), sid, &["--format", "json", "--resolve"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    // TODO(Layer 5): update once --resolve is implemented for JSON inspect output.
    assert!(stderr.contains("planned for layer 5"));
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
