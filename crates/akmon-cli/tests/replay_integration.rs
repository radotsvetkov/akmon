use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_core::{AgentConfig, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::{EventKind, RedbObjectStore, RedbSessionGraph, SessionGraph};
use akmon_models::{CompletionStream, LlmProvider, ModelError, StopReason, StreamEvent};
use akmon_query::AgentSession;
use async_trait::async_trait;
use futures_util::stream;
use serde::Deserialize;
use tempfile::tempdir;
use tokio::sync::mpsc;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::{akmon_bin_path, journal_db_path, open_journal_handle, put_bytes};

#[derive(Debug, Deserialize)]
struct ReplayReportV1 {
    replay_session_id: Option<String>,
    passed: bool,
    divergences: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ReplayInfraError {
    category: String,
    error: String,
}

fn run_replay_with(journal_dir: &Path, session_id: Uuid, extra: &[&str]) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "replay",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra);
    cmd.output().expect("run replay")
}

fn run_replay_without_journal(session_id: &str, extra: &[&str]) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args(["replay", session_id]);
    cmd.args(extra);
    cmd.output().expect("run replay")
}

struct MultiTurnMockProvider {
    call: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for MultiTurnMockProvider {
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
        Ok(Box::pin(stream::iter(vec![
            Ok(StreamEvent::TextDelta {
                text: format!("final response {i}"),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ])))
    }
}

async fn create_replayable_session(journal_dir: &Path, session_id: Uuid, turns: usize) {
    let journal = open_journal_handle(journal_dir, session_id).expect("journal create");
    let cfg = AgentConfig {
        max_iterations: 8,
        confirmation_timeout_secs: 30,
        session_id,
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
        Arc::new(MultiTurnMockProvider {
            call: std::sync::atomic::AtomicUsize::new(0),
        }),
        vec![],
        Arc::new(Sandbox::new(journal_dir)),
        None,
        false,
        journal,
    )
    .expect("session new");

    for i in 0..turns {
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run(format!("turn-{i}"), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
    }
    session.end(None).expect("session end");
}

fn tamper_assistant_turns(journal_dir: &Path, session_id: Uuid) {
    let store = Arc::new(
        RedbObjectStore::open(journal_db_path(journal_dir).as_path()).expect("open store"),
    );
    let mut graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id).expect("reopen graph");
    let history = graph.history().expect("history");
    for (seq, event) in history.iter().map(|(_, e)| (e.sequence, e)) {
        if let EventKind::AssistantTurn {
            tool_calls_hash, ..
        } = &event.kind
        {
            let mut tampered = event.clone();
            tampered.kind = EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), format!("tampered-{seq}").as_bytes()),
                tool_calls_hash: tool_calls_hash.clone(),
            };
            graph
                .overwrite_event_at_sequence_for_testing(seq, tampered)
                .expect("overwrite assistant");
        }
    }
}

#[tokio::test]
async fn t_replay_happy_path_default_mode_exit_zero() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_replayable_session(tmp.path(), sid, 1).await;
    let out = run_replay_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let report: ReplayReportV1 = serde_json::from_slice(&out.stdout).expect("json report");
    assert!(report.passed);
    assert!(report.divergences.is_empty(), "{report:?}");
}

#[tokio::test]
async fn t_replay_happy_path_strict_mode_exit_zero() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_replayable_session(tmp.path(), sid, 1).await;
    let out = run_replay_with(tmp.path(), sid, &["--mode", "strict", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let report: ReplayReportV1 = serde_json::from_slice(&out.stdout).expect("json report");
    assert!(report.passed);
    assert!(report.divergences.is_empty(), "{report:?}");
}

#[tokio::test]
async fn t_replay_synthetic_divergence_exit_one() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_replayable_session(tmp.path(), sid, 1).await;
    tamper_assistant_turns(tmp.path(), sid);
    let out = run_replay_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let report: ReplayReportV1 = serde_json::from_slice(&out.stdout).expect("json report");
    assert!(!report.passed);
    assert!(!report.divergences.is_empty());
}

#[tokio::test]
async fn t_replay_persist_explicit_target_writes_to_specified_dir() {
    let source = tempdir().expect("source");
    let target = tempdir().expect("target");
    let sid = Uuid::new_v4();
    create_replayable_session(source.path(), sid, 1).await;
    let target_str = target.path().display().to_string();
    let out = run_replay_with(
        source.path(),
        sid,
        &["--persist", "--persist-to", &target_str, "--format", "json"],
    );
    // Per extended P11 (Issue 2 fix), SessionStart.config_hash is not compared.
    // Persisted replay with explicit --persist-to should complete cleanly.
    assert_eq!(out.status.code(), Some(0));
    let report: ReplayReportV1 = serde_json::from_slice(&out.stdout).expect("json report");
    let replay_id =
        Uuid::parse_str(report.replay_session_id.as_deref().expect("persisted id")).expect("uuid");
    let target_store = Arc::new(
        RedbObjectStore::open(journal_db_path(target.path()).as_path()).expect("open target store"),
    );
    assert!(RedbSessionGraph::reopen(target_store, replay_id).is_ok());
    assert!(report.passed);
    assert!(report.divergences.is_empty(), "{report:?}");
}

#[test]
fn t_replay_missing_source_session_exit_three() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let out = run_replay_with(tmp.path(), sid, &[]);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("akmon replay:"));
}

#[test]
fn t_replay_persist_to_without_persist_exit_two() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let out = run_replay_with(tmp.path(), sid, &["--persist-to", "/tmp/nowhere"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn t_replay_persist_without_persist_to_exit_two() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let out = run_replay_with(tmp.path(), sid, &["--persist"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn t_replay_invalid_session_id_format_exit_two() {
    let out = run_replay_without_journal("not-a-uuid", &[]);
    assert_eq!(out.status.code(), Some(2));
}

#[tokio::test]
async fn t_replay_json_format_error_produces_valid_json() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let out = run_replay_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(3));
    let err: ReplayInfraError = serde_json::from_slice(&out.stdout).expect("json error");
    assert!(!err.category.is_empty());
    assert!(!err.error.is_empty());
}

#[tokio::test]
async fn t_replay_human_output_truncates_divergences_at_cap() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_replayable_session(tmp.path(), sid, 12).await;
    tamper_assistant_turns(tmp.path(), sid);
    let out = run_replay_with(tmp.path(), sid, &["--format", "human"]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("divergences:"));
    assert!(stdout.contains("[1] event "));
    assert!(stdout.contains("[10] event "));
    assert!(stdout.contains("(and 2 more; use --format json for full list)"));
}

#[tokio::test]
async fn t_replay_journal_flag_overrides_default() {
    let tmp = tempdir().expect("tempdir");
    let custom_journal = tmp.path().join("custom-journal");
    std::fs::create_dir_all(&custom_journal).expect("mkdir");
    let sid = Uuid::new_v4();
    create_replayable_session(custom_journal.as_path(), sid, 1).await;

    let default_state = tmp.path().join("xdg-state-empty");
    std::fs::create_dir_all(&default_state).expect("mkdir default");
    let bin = akmon_bin_path();
    let out = Command::new(bin)
        .args([
            "replay",
            &sid.to_string(),
            "--journal",
            &custom_journal.display().to_string(),
        ])
        .env("XDG_STATE_HOME", &default_state)
        .output()
        .expect("run replay");
    assert_eq!(out.status.code(), Some(0));
}

#[tokio::test]
async fn t_replay_multi_turn_session_exit_zero() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_replayable_session(tmp.path(), sid, 4).await;
    let out = run_replay_with(tmp.path(), sid, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let report: ReplayReportV1 = serde_json::from_slice(&out.stdout).expect("json report");
    assert!(report.passed);
    assert!(report.divergences.is_empty(), "{report:?}");
}
