use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_bundle::{ReadBundleOptions, read_bundle};
use akmon_core::{AgentConfig, Permission, PolicyEngine, PolicyEngineMode, Sandbox};
use akmon_journal::{
    EventKind, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
    referenced_object_hashes_for_kind,
};
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

#[derive(Debug, Deserialize)]
struct BundleExportReportV1 {
    akmon_version: String,
    agef_version: String,
    session_id: String,
    output_path: String,
    events_exported: u64,
    objects_exported: u64,
    bundle_size_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BundleExportError {
    akmon_version: String,
    error: String,
    category: String,
}

fn event_kind_tag(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::SessionStart { .. } => "SessionStart",
        EventKind::UserTurn { .. } => "UserTurn",
        EventKind::ProviderCall { .. } => "ProviderCall",
        EventKind::ToolCall { .. } => "ToolCall",
        EventKind::RetrievalCall { .. } => "RetrievalCall",
        EventKind::PermissionGate { .. } => "PermissionGate",
        EventKind::AssistantTurn { .. } => "AssistantTurn",
        EventKind::SessionEnd { .. } => "SessionEnd",
    }
}

#[test]
fn t_bundle_export_writes_valid_bundle() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("out.akmon");
    let out = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out_path.is_file());
    let file = std::fs::File::open(&out_path).expect("open bundle");
    let parsed = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    assert_eq!(parsed.manifest.session.id, sid.as_hyphenated().to_string());
}

#[test]
fn t_bundle_export_round_trip_via_read_bundle() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let _fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("session.akmon");
    let run = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(run.status.code(), Some(0));

    let store =
        Arc::new(RedbObjectStore::open(journal_db_path(tmp.path()).as_path()).expect("open store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let expected_events: Vec<_> = history.iter().map(|(_, e)| e.clone()).collect();
    let mut expected_objects = std::collections::HashMap::new();
    for (_, ev) in &history {
        for h in referenced_object_hashes_for_kind(&ev.kind) {
            if expected_objects.contains_key(&h) {
                continue;
            }
            let bytes = store.get(&h).expect("get").expect("object present");
            expected_objects.insert(h, bytes.to_vec());
        }
    }

    let file = std::fs::File::open(&out_path).expect("open bundle");
    let contents = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    assert_eq!(
        contents.manifest.session.id,
        sid.as_hyphenated().to_string()
    );
    assert_eq!(
        contents.manifest.event_count as usize,
        expected_events.len()
    );
    assert_eq!(contents.events.len(), expected_events.len());
    for (got, want) in contents.events.iter().zip(expected_events.iter()) {
        assert_eq!(got.sequence, want.sequence);
        assert_eq!(got.parents, want.parents);
        assert_eq!(got.kind, want.kind);
        assert_eq!(
            got.emitted_at.unix_timestamp(),
            want.emitted_at.unix_timestamp(),
            "sequence {} emitted_at second mismatch",
            got.sequence
        );
    }
    assert_eq!(contents.objects, expected_objects);
}

#[test]
fn t_bundle_export_fails_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_present = Uuid::new_v4();
    let sid_missing = Uuid::new_v4();
    create_clean_session(tmp.path(), sid_present);
    let out_path = tmp.path().join("missing.akmon");
    let out = run_bundle_export_with(
        tmp.path(),
        sid_missing,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(out.status.code(), Some(3));
    assert!(!out_path.exists());
}

#[test]
fn t_bundle_export_fails_for_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let empty_journal_dir = tmp.path().join("no_redb");
    std::fs::create_dir_all(&empty_journal_dir).expect("mkdir");
    let sid = Uuid::new_v4();
    let out_path = tmp.path().join("x.akmon");
    let out = run_bundle_export_with(
        &empty_journal_dir,
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(out.status.code(), Some(3));
    assert!(!out_path.exists());
}

#[test]
fn t_bundle_export_fails_for_existing_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("exists.akmon");
    std::fs::write(&out_path, b"occupied").expect("seed file");
    let out = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already exists"), "stderr={stderr}");
}

#[test]
fn t_bundle_export_default_output_path() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let work = tempfile::tempdir_in(tmp.path()).expect("workdir");
    let old_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(work.path()).expect("chdir");
    let out = run_bundle_export_with(tmp.path(), sid, &[]);
    std::env::set_current_dir(&old_cwd).expect("restore cwd");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let default_file = work.path().join(format!("{}.akmon", sid.as_hyphenated()));
    assert!(default_file.is_file());
}

#[test]
fn t_bundle_export_json_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("report.akmon");
    let out = run_bundle_export_with(
        tmp.path(),
        sid,
        &[
            "--format",
            "json",
            "--output",
            &out_path.display().to_string(),
        ],
    );
    assert_eq!(out.status.code(), Some(0));
    let report: BundleExportReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(!report.akmon_version.is_empty());
    assert!(!report.agef_version.is_empty());
    assert_eq!(report.session_id, sid.as_hyphenated().to_string());
    assert!(!report.output_path.is_empty());
    assert_eq!(report.events_exported, 3);
    assert!(report.objects_exported >= 1);
    assert!(report.bundle_size_bytes > 0);
}

const BUNDLE_EXPORT_TOOL: &str = "bundle_export_search";

fn bundle_search_tool_perms() -> &'static [Permission] {
    use std::sync::OnceLock;
    static CELL: OnceLock<Vec<Permission>> = OnceLock::new();
    CELL.get_or_init(|| {
        vec![Permission::ReadFile {
            path: std::path::PathBuf::from("."),
        }]
    })
    .as_slice()
}

struct BundleExportSearchTool;

#[async_trait]
impl Tool for BundleExportSearchTool {
    fn name(&self) -> &str {
        BUNDLE_EXPORT_TOOL
    }
    fn description(&self) -> &str {
        "bundle export integration mock search"
    }
    fn required_permissions(&self) -> &[Permission] {
        bundle_search_tool_perms()
    }
    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::Success {
            content: "BUNDLE_EXPORT_TOOL_OUT".into(),
        }
    }
}

struct BundleExportOneTurnProvider {
    sequences: Vec<Vec<Result<StreamEvent, ModelError>>>,
    call: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for BundleExportOneTurnProvider {
    fn name(&self) -> &str {
        "bundle-export-mock"
    }
    fn context_window_tokens(&self) -> usize {
        200_000
    }
    fn completion_model_id(&self) -> &str {
        "bundle-export-mock-model"
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

fn bundle_export_one_turn_sequences() -> Vec<Vec<Result<StreamEvent, ModelError>>> {
    vec![
        vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            tool_calls: vec![ModelToolCall {
                id: "call-1".into(),
                name: BUNDLE_EXPORT_TOOL.into(),
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
async fn t_bundle_export_full_session_via_real_agent_session() {
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
        Arc::new(BundleExportOneTurnProvider {
            sequences: bundle_export_one_turn_sequences(),
            call: std::sync::atomic::AtomicUsize::new(0),
        }),
        vec![Box::new(BundleExportSearchTool)],
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

    let out_path = tmp.path().join("e2e.akmon");
    let out = run_bundle_export_with(
        tmp.path(),
        sid,
        &["--output", &out_path.display().to_string()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let file = std::fs::File::open(&out_path).expect("open bundle");
    let contents = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    assert_eq!(
        contents.manifest.session.id,
        sid.as_hyphenated().to_string()
    );

    let expected = [
        "SessionStart",
        "UserTurn",
        "ProviderCall",
        "ToolCall",
        "PermissionGate",
        "AssistantTurn",
        "SessionEnd",
    ];
    let kinds: Vec<&str> = contents
        .events
        .iter()
        .map(|e| event_kind_tag(&e.kind))
        .collect();
    for k in expected {
        assert!(kinds.contains(&k), "missing {k} in {kinds:?}");
    }
}

#[test]
fn t_bundle_export_json_error_missing_session_has_category() {
    let tmp = tempdir().expect("tempdir");
    let sid_ok = Uuid::new_v4();
    let sid_bad = Uuid::new_v4();
    create_clean_session(tmp.path(), sid_ok);
    let out_path = tmp.path().join("nope.akmon");
    let out = run_bundle_export_with(
        tmp.path(),
        sid_bad,
        &[
            "--format",
            "json",
            "--output",
            &out_path.display().to_string(),
        ],
    );
    assert_eq!(out.status.code(), Some(3));
    let err: BundleExportError = serde_json::from_slice(&out.stdout).expect("parse err json");
    assert_eq!(err.category, "session_not_found");
    assert!(!err.error.is_empty());
}
