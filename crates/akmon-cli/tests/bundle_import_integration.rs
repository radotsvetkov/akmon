use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_journal::{EventKind, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph};
use akmon_query::journal_contains_session;
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
