use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_journal::{EventKind, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph};
use serde::Deserialize;
use tempfile::tempdir;
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

fn put_bytes(store: &RedbObjectStore, bytes: &[u8]) -> akmon_journal::Hash {
    store.put(bytes).expect("put bytes")
}

fn create_second_clean_session(journal_dir: &Path, session_id: Uuid) {
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("open store"));
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/other-wd"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"y"}"#),
        })
        .expect("append start");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"other-prompt"),
        })
        .expect("append user");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"other-summary")),
        })
        .expect("append end");
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

#[test]
// TODO(Layer 5b-3): Assert exit 0 + successful import after ingestion lands; placeholder is not_implemented.
fn t_bundle_import_without_verify_only_is_placeholder() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let j_src = tmp.path().join("source");
    std::fs::create_dir_all(&j_src).expect("mkdir");
    create_clean_session(&j_src, sid);
    let bundle = tmp.path().join("session.akmon");
    let exp = run_bundle_export_with(&j_src, sid, &["--output", &bundle.display().to_string()]);
    assert_eq!(
        exp.status.code(),
        Some(0),
        "export stderr={}",
        String::from_utf8_lossy(&exp.stderr)
    );
    let j_empty = tmp.path().join("empty_target");
    std::fs::create_dir_all(&j_empty).expect("mkdir");
    let out = run_bundle_import_ingest_attempt(&bundle, &j_empty, &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not implemented") || stderr.contains("layer 5b-3"),
        "stderr={stderr}"
    );
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
// TODO(Layer 5b-3): Expect exit 0 when ingestion lands; placeholder still exits 2 with not_implemented.
fn t_bundle_import_collision_with_rename_to_different_uuid_skips_collision() {
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
    assert_eq!(imp.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&imp.stderr);
    assert!(
        stderr.contains("not implemented") || stderr.contains("layer 5b-3"),
        "expected placeholder not collision: stderr={stderr}"
    );
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
