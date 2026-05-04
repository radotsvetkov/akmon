use std::path::Path;
use std::process::Command;

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
