use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_core::{AgentConfig, Permission, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::RedbObjectStore;
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
use tokio::sync::mpsc;
use uuid::Uuid;

mod common;
use common::*;

fn run_verify(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    run_verify_with(journal_dir, session_id, false, false)
}

fn run_verify_json(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    run_verify_with(journal_dir, session_id, true, false)
}

fn run_verify_verbose(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    run_verify_with(journal_dir, session_id, false, true)
}

fn run_verify_with(
    journal_dir: &std::path::Path,
    session_id: Uuid,
    json: bool,
    verbose: bool,
) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_akmon").expect("CARGO_BIN_EXE_akmon");
    let mut cmd = Command::new(bin);
    cmd.args([
        "verify",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    if json {
        cmd.args(["--format", "json"]);
    }
    if verbose {
        cmd.arg("--verbose");
    }
    cmd.output().expect("run verify")
}

#[derive(Debug, Deserialize)]
struct VerifyReportV1 {
    akmon_version: String,
    agef_version: String,
    session_id: String,
    journal_path: String,
    events_checked: u32,
    objects_checked: u32,
    passed: bool,
    checks_performed: Vec<String>,
    violations: Vec<Violation>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Violation {
    category: String,
    event_hash: Option<String>,
    object_hash: Option<String>,
    message: String,
}

#[derive(Debug, Deserialize)]
struct VerifyError {
    akmon_version: String,
    category: String,
    error: String,
}

fn categories(report: &VerifyReportV1) -> Vec<String> {
    let mut out: Vec<String> = report
        .violations
        .iter()
        .map(|v| v.category.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[test]
fn t_verify_passes_for_clean_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_verify(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("verified: session"));
    assert!(stderr.contains("SessionEnd: present and terminal"));
}

#[test]
fn t_verify_fails_for_corrupted_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    corrupt_fixture_object_bytes(&fixture);

    let out = run_verify(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("verification failed: session"));
    assert!(stderr.contains("object hash mismatches"));
}

#[test]
fn t_verify_fails_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let missing = Uuid::new_v4();
    let out = run_verify(tmp.path(), missing);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open journal"));
}

#[test]
fn t_verify_fails_for_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let missing_dir = tmp.path().join("does-not-exist");
    let sid = Uuid::new_v4();
    let out = run_verify(missing_dir.as_path(), sid);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open journal"));
}

#[test]
fn t_verify_json_output_for_clean_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: VerifyReportV1 = serde_json::from_str(&stdout).expect("parse VerifyReportV1");
    assert!(!parsed.akmon_version.is_empty());
    assert_eq!(parsed.agef_version, "0.1.1");
    assert_eq!(parsed.session_id, sid.to_string());
    assert!(!parsed.journal_path.is_empty());
    assert!(parsed.events_checked > 0);
    assert!(parsed.objects_checked > 0);
    assert!(parsed.passed);
    assert!(!parsed.checks_performed.is_empty());
    assert!(parsed.violations.is_empty());
}

#[test]
fn t_verify_json_output_for_corrupted_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    corrupt_fixture_object_bytes(&fixture);

    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: VerifyReportV1 = serde_json::from_str(&stdout).expect("parse VerifyReportV1");
    assert!(!parsed.akmon_version.is_empty());
    assert!(!parsed.passed);
    let mismatches: Vec<&Violation> = parsed
        .violations
        .iter()
        .filter(|v| v.category == "object_hash_mismatch")
        .collect();
    assert_eq!(mismatches.len(), 1);
    assert!(mismatches[0].object_hash.is_some());
}

#[test]
fn t_verify_json_output_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let missing = Uuid::new_v4();
    let out = run_verify_json(tmp.path(), missing);
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: VerifyError = serde_json::from_str(&stdout).expect("parse VerifyError");
    assert!(!parsed.akmon_version.is_empty());
    assert_eq!(parsed.category, "session_not_found");
    assert!(!parsed.error.is_empty());
}

#[test]
fn t_verify_json_field_stability() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let value: Value = serde_json::from_slice(&out.stdout).expect("parse generic json");
    assert!(value.get("akmon_version").is_some());
    assert!(value.get("agef_version").is_some());
    assert!(value.get("session_id").is_some());
    assert!(value.get("journal_path").is_some());
    assert!(value.get("events_checked").is_some());
    assert!(value.get("objects_checked").is_some());
    assert!(value.get("passed").is_some());
    assert!(value.get("checks_performed").is_some());
    assert!(value.get("violations").is_some());
}

#[test]
fn t_verify_verbose_lists_specific_violations() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let user_event_hash = fixture.user_event_hash.clone();
    let prompt_hash = fixture.prompt_hash.clone();
    let store = Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("open"));
    store
        .remove_object_for_testing(&prompt_hash)
        .expect("remove object");
    drop(store);

    let out = run_verify_verbose(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("missing objects (1):"));
    assert!(stderr.contains(&prompt_hash.to_hex()));
    assert!(stderr.contains(&user_event_hash.to_hex()));
}

#[test]
fn t_verify_verbose_pass_lists_checks_performed() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_verify_verbose(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("checks performed:"));
    assert!(stderr.contains("parent chain: ok"));
    assert!(stderr.contains("sequence: ok"));
    assert!(stderr.contains("event hash recompute: ok"));
    assert!(stderr.contains("object presence: ok"));
    assert!(stderr.contains("object byte re-hash: ok"));
    assert!(stderr.contains("head consistency: ok"));
}

#[test]
fn t_verify_non_verbose_unchanged() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    corrupt_fixture_object_bytes(&fixture);

    let out = run_verify(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("violations:"));
    assert!(stderr.contains("- object hash mismatches: 1"));
    assert!(!stderr.contains("object hash mismatches (1):"));
    assert!(!stderr.contains("checks performed:"));
}

#[test]
fn t_verify_json_unaffected_by_verbose() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);

    let baseline = run_verify_json(tmp.path(), sid);
    assert_eq!(baseline.status.code(), Some(0));
    let with_verbose = run_verify_with(tmp.path(), sid, true, true);
    assert_eq!(with_verbose.status.code(), Some(0));

    let baseline_v: Value = serde_json::from_slice(&baseline.stdout).expect("baseline json");
    let verbose_v: Value = serde_json::from_slice(&with_verbose.stdout).expect("verbose json");
    assert_eq!(baseline_v, verbose_v);
}

#[test]
fn t_verify_report_lists_all_checks_performed() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let parsed: VerifyReportV1 =
        serde_json::from_slice(&out.stdout).expect("parse VerifyReportV1 checks");
    let checks = parsed.checks_performed;
    let expected = [
        "parent_chain",
        "sequence",
        "event_hash_recompute",
        "object_presence",
        "object_byte_rehash",
        "head_consistency",
        "session_end_invariants",
    ];
    for check in expected {
        assert!(
            checks.iter().any(|c| c == check),
            "missing check {check} in {:?}",
            checks
        );
    }
}

#[test]
fn t_verify_detects_missing_session_end() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_missing_end(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(
        categories(&parsed)
            .iter()
            .any(|c| c == "session_end_missing")
    );
}

#[test]
fn t_verify_detects_duplicate_session_end() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_duplicate_end(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(
        categories(&parsed)
            .iter()
            .any(|c| c == "session_end_duplicate")
    );
}

#[test]
fn t_verify_detects_non_terminal_session_end() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_end_not_terminal(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(
        categories(&parsed)
            .iter()
            .any(|c| c == "session_end_not_terminal")
    );
}

#[test]
fn t_verify_detects_event_hash_mismatch() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_event_hash_mismatch(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(
        categories(&parsed)
            .iter()
            .any(|c| c == "event_hash_mismatch")
    );
}

#[test]
fn t_verify_detects_parent_chain_break() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_parent_chain_break(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(categories(&parsed).iter().any(|c| c == "parent_chain"));
}

#[test]
fn t_verify_detects_head_mismatch() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_head_mismatch(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(categories(&parsed).iter().any(|c| c == "head_mismatch"));
}

#[test]
fn t_verify_detects_multiple_violations_in_single_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_session_multi_violation(tmp.path(), sid);
    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    let cats = categories(&parsed);
    assert!(cats.iter().any(|c| c == "missing_object"));
    assert!(cats.iter().any(|c| c == "session_end_duplicate"));
}

const TOOL_NAME: &str = "search_like";
const TOOL_OUTPUT: &str = "TOOL_OUTPUT_PRED";

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
async fn t_verify_full_session_via_real_agent_session() {
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

    let out = run_verify_json(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let parsed: VerifyReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(parsed.passed);
    assert!(parsed.violations.is_empty());
    let expected = [
        "parent_chain",
        "sequence",
        "event_hash_recompute",
        "object_presence",
        "object_byte_rehash",
        "head_consistency",
        "session_end_invariants",
    ];
    for check in expected {
        assert!(parsed.checks_performed.iter().any(|c| c == check));
    }
}
