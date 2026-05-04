use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_journal::{
    EventKind, HashAlgorithm, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
};
use serde::Deserialize;
use serde_json::Value;
use tempfile::tempdir;
use uuid::Uuid;

fn journal_db_path(dir: &Path) -> std::path::PathBuf {
    dir.join("journal.redb")
}

fn put_bytes(store: &RedbObjectStore, bytes: &[u8]) -> akmon_journal::Hash {
    store.put(bytes).expect("put bytes")
}

fn create_clean_session(journal_dir: &Path, session_id: Uuid) {
    std::fs::create_dir_all(journal_dir).expect("mkdir journal dir");
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).expect("create store"),
    );
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("append start");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"hello"),
        })
        .expect("append user");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("append end");
}

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
    journal_dir: &Path,
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
    create_clean_session(tmp.path(), sid);

    let db_path = journal_db_path(tmp.path());
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("reopen store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let prompt_hash = history
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    store
        .overwrite_object_bytes_for_testing(&prompt_hash, b"corrupted-object")
        .expect("overwrite object");
    drop(graph);
    drop(store);

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
    assert!(parsed.violations.is_empty());
}

#[test]
fn t_verify_json_output_for_corrupted_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);

    let db_path = journal_db_path(tmp.path());
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("reopen store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let prompt_hash = history
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    store
        .overwrite_object_bytes_for_testing(&prompt_hash, b"corrupted-object")
        .expect("overwrite object");
    drop(graph);
    drop(store);

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
    assert!(value.get("violations").is_some());
}

#[test]
fn t_verify_verbose_lists_specific_violations() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);

    let db_path = journal_db_path(tmp.path());
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("reopen store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let prompt_hash = history
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    store
        .overwrite_object_bytes_for_testing(&prompt_hash, b"corrupted-object")
        .expect("overwrite object");
    drop(graph);
    drop(store);

    let out = run_verify_verbose(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("object hash mismatches (1):"));
    assert!(stderr.contains(&prompt_hash.to_hex()));
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
    create_clean_session(tmp.path(), sid);

    let db_path = journal_db_path(tmp.path());
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("reopen store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let prompt_hash = history
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    store
        .overwrite_object_bytes_for_testing(&prompt_hash, b"corrupted-object")
        .expect("overwrite object");
    drop(graph);
    drop(store);

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
