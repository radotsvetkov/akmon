use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_core::{AgentConfig, Permission, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::{EventKind, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph};
use akmon_models::{
    CompletionStream, LlmProvider, ModelError, ModelToolCall, StopReason, StreamEvent,
};
use akmon_query::AgentSession;
use akmon_query::journal_contains_session;
use akmon_tools::{Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use futures_util::stream;
use serde::Deserialize;
use serde_json::Value;
use tempfile::tempdir;
use tokio::sync::mpsc;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::*;

fn run_bundle_export_with(
    journal_dir: &Path,
    session_id: Uuid,
    extra: &[&str],
) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "bundle",
        "export",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra);
    cmd.output().expect("run bundle export")
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
        "--verify-only",
    ]);
    cmd.args(extra);
    cmd.output().expect("run bundle import")
}

fn run_bundle_import_ingest_attempt(
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
    cmd.output().expect("run bundle import ingest")
}

fn run_verify_with(journal_dir: &Path, session_id: Uuid, extra: &[&str]) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    let sid_text = session_id.to_string();
    cmd.args([
        "verify",
        sid_text.as_str(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra);
    cmd.output().expect("run verify")
}

fn run_inspect_with(journal_dir: &Path, session_id: Uuid, extra: &[&str]) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "inspect",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra);
    cmd.output().expect("run inspect")
}

const TOOL_NAME: &str = "search_workspace";
const TOOL_OUTPUT: &str = "search: 2 results";

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
        &[]
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

async fn create_real_agent_session_journal(journal_dir: &Path, sid: Uuid) {
    let journal = open_journal_handle(journal_dir, sid).expect("journal create");
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
        Arc::new(Sandbox::new(journal_dir)),
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
}

fn history_hashes(journal_dir: &Path, sid: Uuid) -> Vec<String> {
    let store = Arc::new(
        RedbObjectStore::open(journal_dir.join("journal.redb").as_path()).expect("open store"),
    );
    let graph = RedbSessionGraph::reopen(store, sid).expect("reopen graph");
    graph
        .history()
        .expect("history")
        .into_iter()
        .map(|(h, _)| h.to_hex())
        .collect()
}

fn parse_import_human_counts(stderr: &str) -> Option<(u64, u64, u64)> {
    // NOTE: This parser is coupled to run_bundle_import's human output format. If the wording in
    // main.rs changes (for example, "existing in store" -> "already present"), this parser must
    // be updated. Consider this when polishing CLI output in Phase 7.
    let mut events = None;
    let mut objects_new = None;
    let mut objects_existing = None;
    for line in stderr.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("events: ") {
            events = rest.parse::<u64>().ok();
        }
        if let Some(rest) = t.strip_prefix("objects: ") {
            // "31 (28 new, 3 existing in store)"
            let total_sep = rest.find(' ')?;
            let _total = rest[..total_sep].parse::<u64>().ok()?;
            let inner = rest.split('(').nth(1)?.trim_end_matches(')');
            let mut parts = inner.split(',');
            let new_part = parts.next()?.trim();
            let existing_part = parts.next()?.trim();
            objects_new = new_part.strip_suffix(" new")?.parse::<u64>().ok();
            objects_existing = existing_part
                .strip_suffix(" existing in store")?
                .parse::<u64>()
                .ok();
        }
    }
    Some((events?, objects_new?, objects_existing?))
}

fn create_second_clean_session(journal_dir: &Path, session_id: Uuid) {
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("open store"));
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: store.put(b"/workspace-two").expect("cwd hash"),
            config_hash: store.put(br#"{"model":"m2"}"#).expect("config hash"),
        })
        .expect("append start");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: store.put(b"prompt-two").expect("prompt hash"),
        })
        .expect("append turn");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(store.put(b"summary-two").expect("summary hash")),
        })
        .expect("append end");
}

#[derive(Debug, Deserialize)]
struct BundleVerifyReportV1 {
    akmon_version: String,
    agef_version: String,
    bundle_path: String,
    session_id: String,
    events_in_bundle: u64,
    objects_in_bundle: u64,
    passed: bool,
    violations: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct BundleImportInfraErrorV1 {
    akmon_version: String,
    error: String,
    category: String,
    colliding_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BundleImportReportV1 {
    akmon_version: String,
    agef_version: String,
    bundle_path: String,
    original_session_id: String,
    imported_session_id: String,
    events_imported: u64,
    objects_total: u64,
    objects_new: u64,
    objects_existing: u64,
    journal_path: String,
}

#[test]
fn t_bundle_import_succeeds_for_clean_bundle() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    create_clean_session(&src, sid);
    let bundle = tmp.path().join("session.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );
    let out = run_bundle_import_ingest_attempt(&bundle, &dst, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(journal_contains_session(&dst, sid).expect("contains imported sid"));
    let verify = run_verify_with(&dst, sid, &[]);
    assert_eq!(
        verify.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
}

#[test]
fn t_bundle_import_with_rename_to_succeeds() {
    let tmp = tempdir().expect("tempdir");
    let sid_src = Uuid::new_v4();
    let sid_dst = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    create_clean_session(&src, sid_src);
    let bundle = tmp.path().join("session.akmon");
    assert!(
        run_bundle_export_with(&src, sid_src, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );
    let imp =
        run_bundle_import_ingest_attempt(&bundle, &dst, &["--rename-to", &sid_dst.to_string()]);
    assert_eq!(
        imp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp.stderr)
    );
    assert!(journal_contains_session(&dst, sid_dst).expect("contains renamed sid"));
    assert!(!journal_contains_session(&dst, sid_src).expect("does not contain original sid"));
    let verify = run_verify_with(&dst, sid_dst, &[]);
    assert_eq!(
        verify.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
}

#[test]
fn t_bundle_import_creates_journal_if_missing() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("missing_journal_dir");
    std::fs::create_dir_all(&src).expect("mkdir src");
    create_clean_session(&src, sid);
    let bundle = tmp.path().join("session.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );
    assert!(!dst.join("journal.redb").exists());
    let out = run_bundle_import_ingest_attempt(&bundle, &dst, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dst.join("journal.redb").exists());
}

#[test]
fn t_bundle_import_object_collision_matching_bytes() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    let fixture = create_clean_session(&src, sid);
    create_clean_session(&dst, Uuid::new_v4());
    {
        let dst_store =
            Arc::new(RedbObjectStore::open(dst.join("journal.redb").as_path()).expect("open dst"));
        dst_store.put(b"hello").expect("pre-seed matching object");
    }
    let bundle = tmp.path().join("session.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );
    let out = run_bundle_import_ingest_attempt(&bundle, &dst, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: BundleImportReportV1 =
        serde_json::from_slice(&out.stdout).expect("parse import json");
    assert_eq!(report.imported_session_id, sid.to_string());
    assert!(
        report.objects_existing > 0,
        "expected at least one existing object"
    );
    let verify = run_verify_with(&dst, sid, &[]);
    assert_eq!(
        verify.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(fixture.prompt_hash.to_hex().len() > 8);
}

#[test]
fn t_bundle_import_object_collision_mismatching_bytes() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    let fixture = create_clean_session(&src, sid);
    create_clean_session(&dst, Uuid::new_v4());
    {
        let dst_store =
            Arc::new(RedbObjectStore::open(dst.join("journal.redb").as_path()).expect("open dst"));
        dst_store.put(b"hello").expect("pre-seed object");
        dst_store
            .overwrite_object_bytes_for_testing(&fixture.prompt_hash, b"corrupt-local-object")
            .expect("corrupt local object bytes");
    }
    let bundle = tmp.path().join("session.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );
    let out = run_bundle_import_ingest_attempt(&bundle, &dst, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(3));
    let err: BundleImportInfraErrorV1 =
        serde_json::from_slice(&out.stdout).expect("parse error json");
    assert_eq!(err.category, "local_store_corrupt");
}

#[test]
fn t_bundle_import_collision_without_rename_exits_2() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("b.akmon");
    let exp = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(exp.status.code(), Some(0));
    let imp = run_bundle_import_ingest_attempt(&out_path, tmp.path(), &[]);
    assert_eq!(imp.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&imp.stderr);
    assert!(stderr.contains("already contains"), "stderr={stderr}");
}

#[test]
fn t_bundle_import_collision_without_rename_json_category() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("b.akmon");
    assert!(
        run_bundle_export_with(
            tmp.path(),
            sid,
            &["--output", &out_path.display().to_string()],
        )
        .status
        .success()
    );
    let imp = run_bundle_import_ingest_attempt(&out_path, tmp.path(), &["--format", "json"]);
    assert_eq!(imp.status.code(), Some(2));
    let err: BundleImportInfraErrorV1 = serde_json::from_slice(&imp.stdout).expect("parse json");
    assert_eq!(err.category, "session_id_collision");
    assert!(!err.error.is_empty());
    assert_eq!(err.colliding_session_id, Some(sid.to_string()));
    assert!(!err.akmon_version.is_empty());
}

#[test]
fn t_bundle_import_collision_with_rename_to_different_uuid_proceeds() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("b.akmon");
    assert!(
        run_bundle_export_with(
            tmp.path(),
            sid,
            &["--output", &out_path.display().to_string()],
        )
        .status
        .success()
    );
    let fresh = Uuid::new_v4();
    let imp = run_bundle_import_ingest_attempt(
        &out_path,
        tmp.path(),
        &["--rename-to", &fresh.to_string()],
    );
    assert_eq!(
        imp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp.stderr)
    );
    assert!(journal_contains_session(tmp.path(), fresh).expect("contains renamed sid"));
}

#[test]
fn t_bundle_import_collision_with_rename_to_existing_uuid_exits_2() {
    let tmp = tempdir().expect("tempdir");
    let sid_x = Uuid::new_v4();
    let sid_y = Uuid::new_v4();
    create_clean_session(tmp.path(), sid_x);
    create_second_clean_session(tmp.path(), sid_y);
    let out_path = tmp.path().join("b.akmon");
    assert!(
        run_bundle_export_with(
            tmp.path(),
            sid_x,
            &["--output", &out_path.display().to_string()],
        )
        .status
        .success()
    );
    let imp = run_bundle_import_ingest_attempt(
        &out_path,
        tmp.path(),
        &["--rename-to", &sid_y.to_string(), "--format", "json"],
    );
    assert_eq!(imp.status.code(), Some(2));
    let err: BundleImportInfraErrorV1 = serde_json::from_slice(&imp.stdout).expect("parse json");
    assert_eq!(err.category, "session_id_collision");
    assert!(!err.error.is_empty());
    assert_eq!(err.colliding_session_id, Some(sid_y.to_string()));
}

#[test]
fn t_bundle_import_json_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    create_clean_session(&src, sid);
    let out_path = tmp.path().join("session.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &out_path.display().to_string()])
            .status
            .success()
    );

    let imp = run_bundle_import_ingest_attempt(&out_path, &dst, &["--format", "json"]);
    assert_eq!(imp.status.code(), Some(0));
    let report: BundleImportReportV1 = serde_json::from_slice(&imp.stdout).expect("parse json");
    assert!(!report.akmon_version.is_empty());
    assert!(!report.agef_version.is_empty());
    assert!(!report.bundle_path.is_empty());
    assert!(!report.journal_path.is_empty());
    assert_eq!(report.original_session_id, sid.to_string());
    assert_eq!(report.imported_session_id, sid.to_string());
    assert_eq!(report.events_imported, 3);
    assert_eq!(
        report.objects_total,
        report.objects_new + report.objects_existing
    );
}

#[tokio::test]
async fn t_bundle_import_round_trip_full_session_via_real_agent_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    create_real_agent_session_journal(&src, sid).await;

    let bundle = tmp.path().join("full.akmon");
    let exp = run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()]);
    assert_eq!(
        exp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&exp.stderr)
    );

    let imp = run_bundle_import_ingest_attempt(&bundle, &dst, &[]);
    assert_eq!(
        imp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp.stderr)
    );

    let verify = run_verify_with(&dst, sid, &[]);
    assert_eq!(
        verify.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );

    let src_hashes = history_hashes(&src, sid);
    let dst_hashes = history_hashes(&dst, sid);
    assert_eq!(
        src_hashes, dst_hashes,
        "event hash lists must match exactly"
    );
}

#[test]
fn t_bundle_import_then_inspect_displays_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    create_clean_session(&src, sid);
    let bundle = tmp.path().join("inspect.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );
    assert!(
        run_bundle_import_ingest_attempt(&bundle, &dst, &[])
            .status
            .success()
    );

    let out = run_inspect_with(&dst, sid, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for kind in ["SessionStart", "UserTurn", "SessionEnd"] {
        assert!(stdout.contains(kind), "missing {kind} in inspect output");
    }
}

#[test]
fn t_bundle_import_idempotency_via_rename_to() {
    let tmp = tempdir().expect("tempdir");
    let sid_src = Uuid::new_v4();
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    create_clean_session(&src, sid_src);
    let bundle = tmp.path().join("idem.akmon");
    assert!(
        run_bundle_export_with(&src, sid_src, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );

    let imp_a =
        run_bundle_import_ingest_attempt(&bundle, &dst, &["--rename-to", &sid_a.to_string()]);
    assert_eq!(
        imp_a.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp_a.stderr)
    );
    let imp_b =
        run_bundle_import_ingest_attempt(&bundle, &dst, &["--rename-to", &sid_b.to_string()]);
    assert_eq!(
        imp_b.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp_b.stderr)
    );

    assert!(journal_contains_session(&dst, sid_a).expect("contains A"));
    assert!(journal_contains_session(&dst, sid_b).expect("contains B"));

    let verify_a = run_verify_with(&dst, sid_a, &[]);
    let verify_b = run_verify_with(&dst, sid_b, &[]);
    assert_eq!(
        verify_a.status.code(),
        Some(0),
        "A stderr={}",
        String::from_utf8_lossy(&verify_a.stderr)
    );
    assert_eq!(
        verify_b.status.code(),
        Some(0),
        "B stderr={}",
        String::from_utf8_lossy(&verify_b.stderr)
    );

    assert_eq!(history_hashes(&dst, sid_a), history_hashes(&dst, sid_b));
}

#[test]
fn t_bundle_import_human_and_json_describe_same_outcome() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let src = tmp.path().join("src");
    let dst_h = tmp.path().join("dst_h");
    let dst_j = tmp.path().join("dst_j");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::create_dir_all(&dst_h).expect("mkdir dst_h");
    std::fs::create_dir_all(&dst_j).expect("mkdir dst_j");
    create_clean_session(&src, sid);
    let bundle = tmp.path().join("human_json.akmon");
    assert!(
        run_bundle_export_with(&src, sid, &["--output", &bundle.display().to_string()])
            .status
            .success()
    );

    let human = run_bundle_import_ingest_attempt(&bundle, &dst_h, &[]);
    assert_eq!(human.status.code(), Some(0));
    let human_stderr = String::from_utf8_lossy(&human.stderr);
    let (events_h, new_h, existing_h) =
        parse_import_human_counts(&human_stderr).expect("parse human counts");

    let json = run_bundle_import_ingest_attempt(&bundle, &dst_j, &["--format", "json"]);
    assert_eq!(json.status.code(), Some(0));
    let report: BundleImportReportV1 =
        serde_json::from_slice(&json.stdout).expect("parse json report");

    assert_eq!(report.events_imported, events_h);
    assert_eq!(report.objects_new, new_h);
    assert_eq!(report.objects_existing, existing_h);
}

#[test]
fn t_bundle_import_verify_only_passes_on_exported_bundle() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("session.akmon");
    let exp = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(
        exp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&exp.stderr)
    );

    let imp = run_bundle_import_with(&out_path, tmp.path(), &[]);
    assert_eq!(
        imp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp.stderr)
    );
}

#[test]
fn t_bundle_import_verify_only_json_report() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("session.akmon");
    let exp = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(exp.status.code(), Some(0));

    let imp = run_bundle_import_with(&out_path, tmp.path(), &["--format", "json"]);
    assert_eq!(imp.status.code(), Some(0));
    let report: BundleVerifyReportV1 = serde_json::from_slice(&imp.stdout).expect("parse json");
    assert!(report.passed);
    assert!(report.violations.is_empty());
    assert_eq!(report.session_id, sid.as_hyphenated().to_string());
    assert_eq!(report.events_in_bundle, 3);
    assert!(report.objects_in_bundle >= 1);
    assert!(!report.akmon_version.is_empty());
    assert!(!report.agef_version.is_empty());
    assert!(!report.bundle_path.is_empty());
}
