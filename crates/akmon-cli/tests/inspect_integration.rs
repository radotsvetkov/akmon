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
use serde_json::Value;
use tempfile::tempdir;
use time::{Duration, OffsetDateTime};
use tokio::sync::mpsc;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::*;

fn run_inspect(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_akmon").expect("CARGO_BIN_EXE_akmon");
    Command::new(bin)
        .args([
            "inspect",
            &session_id.to_string(),
            "--journal",
            &journal_dir.display().to_string(),
        ])
        .output()
        .expect("run inspect")
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
