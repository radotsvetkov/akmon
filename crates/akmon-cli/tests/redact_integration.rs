use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use akmon_bundle::{ReadBundleOptions, is_sentinel, read_bundle};
use akmon_journal::{
    EventKind, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
    referenced_object_hashes_for_kind,
};
use serde::Deserialize;
use serde_json::Value;
use tempfile::tempdir;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::*;

fn run_redact_with(
    journal_dir: &Path,
    session_id: Uuid,
    output_path: &Path,
    objects: &[&str],
    reason: &str,
    extra: &[&str],
) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "redact",
        &session_id.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
        "--output",
        &output_path.display().to_string(),
    ]);
    for object in objects {
        cmd.args(["--object", object]);
    }
    cmd.args(["--reason", reason]);
    cmd.args(extra);
    cmd.output().expect("run redact")
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
    ]);
    cmd.args(extra);
    cmd.output().expect("run bundle import")
}

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

fn run_verify_with(journal_dir: &Path, session_id: Uuid, extra: &[&str]) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "verify",
        &session_id.to_string(),
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

fn source_session_head(journal_dir: &Path, session_id: Uuid) -> String {
    let store = Arc::new(
        RedbObjectStore::open(journal_db_path(journal_dir).as_path()).expect("open store"),
    );
    let graph = RedbSessionGraph::reopen(store, session_id).expect("reopen graph");
    graph
        .head()
        .expect("head")
        .expect("non-empty head")
        .to_hex()
}

#[derive(Debug, Deserialize)]
struct RedactReportV1 {
    akmon_version: String,
    agef_version: String,
    source_session_id: String,
    source_head: String,
    derivative_head: String,
    events_in_session: u64,
    events_rewritten_count: u64,
    objects_redacted_count: u64,
    redacted_objects: Vec<RedactedObjectEntry>,
    output_path: String,
    bundle_size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct RedactedObjectEntry {
    original_hash: String,
    sentinel_hash: String,
    original_size: u64,
}

#[derive(Debug, Deserialize)]
struct RedactError {
    akmon_version: String,
    error: String,
    category: String,
    invalid_object_hash: Option<String>,
    missing_object_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BundleVerifyReportV1 {
    passed: bool,
}

fn parse_redact_human_counts(stderr: &str) -> Option<(u64, u64)> {
    // NOTE: Coupled to redact human output wording in main.rs.
    // If labels change during Phase 7 polish, update this parser.
    let mut events_rewritten = None;
    let mut objects_redacted = None;
    for line in stderr.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("events rewritten: ") {
            events_rewritten = rest.parse::<u64>().ok();
        }
        if let Some(rest) = t.strip_prefix("objects redacted: ") {
            objects_redacted = rest.parse::<u64>().ok();
        }
    }
    Some((events_rewritten?, objects_redacted?))
}

fn contains_redacted_reason(value: &Value, reason: &str) -> bool {
    match value {
        Value::Object(map) => {
            let has_redacted_true = map
                .iter()
                .any(|(k, v)| k.ends_with("_redacted") && v.as_bool() == Some(true));
            let has_reason = map
                .iter()
                .any(|(k, v)| k.ends_with("_redaction_reason") && v.as_str() == Some(reason));
            if has_redacted_true && has_reason {
                return true;
            }
            map.values().any(|v| contains_redacted_reason(v, reason))
        }
        Value::Array(arr) => arr.iter().any(|v| contains_redacted_reason(v, reason)),
        _ => false,
    }
}

#[test]
fn t_redact_writes_valid_derivative_bundle() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("redacted.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "PII removal",
        &[],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file = std::fs::File::open(&out_path).expect("open bundle");
    let parsed = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    assert_eq!(parsed.manifest.session.id, sid.as_hyphenated().to_string());
}

#[test]
fn t_redact_substitutes_sentinel_correctly() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("redacted.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "sensitive prompt",
        &[],
    );
    assert_eq!(out.status.code(), Some(0));

    let file = std::fs::File::open(&out_path).expect("open bundle");
    let parsed = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    let mut prompt_hash_after = None;
    for event in &parsed.events {
        if let EventKind::UserTurn { prompt_hash } = &event.kind {
            prompt_hash_after = Some(prompt_hash.clone());
        }
    }
    let prompt_hash_after = prompt_hash_after.expect("user turn prompt hash");
    assert_ne!(prompt_hash_after, fixture.prompt_hash);
    let sentinel_bytes = parsed
        .objects
        .get(&prompt_hash_after)
        .expect("sentinel object present");
    assert!(is_sentinel(sentinel_bytes.as_slice()));
}

#[test]
fn t_redact_preserves_unrelated_objects() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("redacted.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "remove prompt",
        &[],
    );
    assert_eq!(out.status.code(), Some(0));

    let store =
        Arc::new(RedbObjectStore::open(journal_db_path(tmp.path()).as_path()).expect("open store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let untouched = history
        .iter()
        .flat_map(|(_, ev)| referenced_object_hashes_for_kind(&ev.kind))
        .find(|h| *h != fixture.prompt_hash)
        .expect("at least one untouched hash");
    let original_bytes = store
        .get(&untouched)
        .expect("get object")
        .expect("object present")
        .to_vec();

    let file = std::fs::File::open(&out_path).expect("open bundle");
    let parsed = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    let copied_bytes = parsed
        .objects
        .get(&untouched)
        .expect("untouched object present in derivative");
    assert_eq!(copied_bytes.as_slice(), original_bytes.as_slice());
}

#[test]
fn t_redact_preserves_session_id() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("redacted.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "id-check",
        &[],
    );
    assert_eq!(out.status.code(), Some(0));
    let file = std::fs::File::open(&out_path).expect("open bundle");
    let parsed = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    assert_eq!(parsed.manifest.session.id, sid.as_hyphenated().to_string());
}

#[test]
fn t_redact_recomputes_head() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let source_head = source_session_head(tmp.path(), sid);
    let out_path = tmp.path().join("redacted.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "head-change",
        &[],
    );
    assert_eq!(out.status.code(), Some(0));
    let file = std::fs::File::open(&out_path).expect("open bundle");
    let parsed = read_bundle(file, &ReadBundleOptions::default()).expect("read_bundle");
    assert_ne!(parsed.manifest.session.head, source_head);
}

#[test]
fn t_redact_imports_back_cleanly() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("redacted.akmon");
    let red = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "import-check",
        &[],
    );
    assert_eq!(red.status.code(), Some(0));

    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir dst");
    let renamed = Uuid::new_v4();
    let imp = run_bundle_import_with(
        out_path.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(
        imp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&imp.stderr)
    );
    let verify = run_verify_with(dst.as_path(), renamed, &[]);
    assert_eq!(
        verify.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
}

#[test]
fn t_redact_fails_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_present = Uuid::new_v4();
    create_clean_session(tmp.path(), sid_present);
    let sid_missing = Uuid::new_v4();
    let out = run_redact_with(
        tmp.path(),
        sid_missing,
        tmp.path().join("missing.akmon").as_path(),
        &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "missing-session",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn t_redact_fails_for_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let missing = tmp.path().join("no_journal");
    std::fs::create_dir_all(&missing).expect("mkdir");
    let out = run_redact_with(
        missing.as_path(),
        Uuid::new_v4(),
        tmp.path().join("x.akmon").as_path(),
        &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "missing-journal",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn t_redact_fails_for_existing_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("exists.akmon");
    std::fs::write(&out_path, b"occupied").expect("seed");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "exists",
        &[],
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn t_redact_fails_for_invalid_object_hash() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_redact_with(
        tmp.path(),
        sid,
        tmp.path().join("invalid.akmon").as_path(),
        &["zz-not-hex"],
        "invalid-hash",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(2));
    let err: RedactError = serde_json::from_slice(&out.stdout).expect("parse error");
    assert!(!err.akmon_version.is_empty());
    assert!(!err.error.is_empty());
    assert_eq!(err.category, "invalid_object_hash");
    assert_eq!(err.invalid_object_hash, Some("zz-not-hex".to_owned()));
}

#[test]
fn t_redact_fails_for_object_not_in_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_redact_with(
        tmp.path(),
        sid,
        tmp.path().join("not_in_session.akmon").as_path(),
        &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "not-in-session",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(2));
    let err: RedactError = serde_json::from_slice(&out.stdout).expect("parse error");
    assert!(!err.akmon_version.is_empty());
    assert!(!err.error.is_empty());
    assert_eq!(err.category, "object_not_in_session");
    assert_eq!(
        err.missing_object_hash,
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned())
    );
}

#[test]
fn t_redact_json_output() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let out_path = tmp.path().join("report.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "json-report",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(0));
    let report: RedactReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert!(!report.akmon_version.is_empty());
    assert!(!report.agef_version.is_empty());
    assert_eq!(report.source_session_id, sid.to_string());
    assert!(!report.source_head.is_empty());
    assert!(!report.derivative_head.is_empty());
    assert_eq!(report.events_in_session, 3);
    assert!(report.events_rewritten_count >= 1);
    assert_eq!(report.objects_redacted_count, 1);
    assert_eq!(report.redacted_objects.len(), 1);
    assert_eq!(
        report.redacted_objects[0].original_hash,
        fixture.prompt_hash.to_hex()
    );
    assert!(!report.redacted_objects[0].sentinel_hash.is_empty());
    assert!(report.redacted_objects[0].original_size > 0);
    assert_eq!(
        PathBuf::from(report.output_path),
        dunce::canonicalize(out_path).expect("canonical output")
    );
    assert!(report.bundle_size_bytes > 0);
}

#[test]
fn t_redact_multiple_objects_one_invocation() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let store =
        Arc::new(RedbObjectStore::open(journal_db_path(tmp.path()).as_path()).expect("open store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let summary_hash = history
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::SessionEnd {
                summary_hash: Some(h),
            } => Some(h.clone()),
            _ => None,
        })
        .expect("summary hash");
    drop(graph);
    drop(store);
    let out_path = tmp.path().join("multi.akmon");
    let out = run_redact_with(
        tmp.path(),
        sid,
        out_path.as_path(),
        &[&fixture.prompt_hash.to_hex(), &summary_hash.to_hex()],
        "multi-redaction",
        &["--format", "json"],
    );
    assert_eq!(out.status.code(), Some(0));
    let report: RedactReportV1 = serde_json::from_slice(&out.stdout).expect("parse report");
    assert_eq!(report.objects_redacted_count, 2);
    let redacted_originals: HashSet<String> = report
        .redacted_objects
        .iter()
        .map(|e| e.original_hash.clone())
        .collect();
    assert!(redacted_originals.contains(&fixture.prompt_hash.to_hex()));
    assert!(redacted_originals.contains(&summary_hash.to_hex()));
}

#[test]
fn t_redact_derivative_bundle_passes_verify_only() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let bundle = tmp.path().join("derivative.akmon");
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "verify-only-check",
        &[],
    );
    assert_eq!(redact.status.code(), Some(0));
    let verify_only = run_bundle_import_with(
        bundle.as_path(),
        tmp.path(),
        &["--verify-only", "--format", "json"],
    );
    assert_eq!(
        verify_only.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&verify_only.stderr)
    );
    let report: BundleVerifyReportV1 =
        serde_json::from_slice(&verify_only.stdout).expect("parse verify-only json");
    assert!(report.passed);
}

#[test]
fn t_redact_imported_session_can_be_exported() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let derivative = tmp.path().join("redacted.akmon");
    let redact = run_redact_with(
        tmp.path(),
        sid,
        derivative.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "re-export-check",
        &["--format", "json"],
    );
    assert_eq!(redact.status.code(), Some(0));
    let report: RedactReportV1 = serde_json::from_slice(&redact.stdout).expect("redact report");
    let sentinel_hash = report.redacted_objects[0].sentinel_hash.clone();

    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(&dst).expect("mkdir");
    let renamed = Uuid::new_v4();
    let import = run_bundle_import_with(
        derivative.as_path(),
        dst.as_path(),
        &["--rename-to", &renamed.to_string()],
    );
    assert_eq!(import.status.code(), Some(0));

    let re_export = dst.join("reexport.akmon");
    let export = run_bundle_export_with(
        dst.as_path(),
        renamed,
        &["--output", &re_export.display().to_string()],
    );
    assert_eq!(
        export.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    let file = std::fs::File::open(&re_export).expect("open re-exported bundle");
    let contents =
        read_bundle(file, &ReadBundleOptions::default()).expect("read re-exported bundle");
    let sentinel_entry = contents
        .objects
        .iter()
        .find(|(hash, _)| hash.to_hex() == sentinel_hash)
        .expect("sentinel hash present in re-export");
    assert!(is_sentinel(sentinel_entry.1.as_slice()));
}

#[test]
fn t_redact_round_trip_preserves_sentinels() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);
    let reason = "round-trip-redaction";
    let bundle_a = tmp.path().join("bundle_a.akmon");
    let redact = run_redact_with(
        tmp.path(),
        sid,
        bundle_a.as_path(),
        &[&fixture.prompt_hash.to_hex()],
        reason,
        &[],
    );
    assert_eq!(redact.status.code(), Some(0));

    let journal_a = tmp.path().join("journal_a");
    std::fs::create_dir_all(&journal_a).expect("mkdir a");
    let sid_a = Uuid::new_v4();
    let import_a = run_bundle_import_with(
        bundle_a.as_path(),
        journal_a.as_path(),
        &["--rename-to", &sid_a.to_string()],
    );
    assert_eq!(import_a.status.code(), Some(0));

    let bundle_b = tmp.path().join("bundle_b.akmon");
    let export_b = run_bundle_export_with(
        journal_a.as_path(),
        sid_a,
        &["--output", &bundle_b.display().to_string()],
    );
    assert_eq!(export_b.status.code(), Some(0));

    let journal_b = tmp.path().join("journal_b");
    std::fs::create_dir_all(&journal_b).expect("mkdir b");
    let sid_b = Uuid::new_v4();
    let import_b = run_bundle_import_with(
        bundle_b.as_path(),
        journal_b.as_path(),
        &["--rename-to", &sid_b.to_string()],
    );
    assert_eq!(import_b.status.code(), Some(0));

    let inspect = run_inspect_with(
        journal_b.as_path(),
        sid_b,
        &["--resolve", "--format", "json"],
    );
    assert_eq!(
        inspect.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let value: Value = serde_json::from_slice(&inspect.stdout).expect("inspect json");
    assert!(contains_redacted_reason(&value, reason));
}

#[test]
fn t_redact_human_and_json_describe_same_outcome() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    let fixture = create_clean_session(tmp.path(), sid);

    let human_out = run_redact_with(
        tmp.path(),
        sid,
        tmp.path().join("human.akmon").as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "parity-check",
        &[],
    );
    assert_eq!(
        human_out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&human_out.stderr)
    );
    let (events_h, objects_h) =
        parse_redact_human_counts(&String::from_utf8_lossy(&human_out.stderr))
            .expect("parse human redact counts");

    let json_out = run_redact_with(
        tmp.path(),
        sid,
        tmp.path().join("json.akmon").as_path(),
        &[&fixture.prompt_hash.to_hex()],
        "parity-check",
        &["--format", "json"],
    );
    assert_eq!(json_out.status.code(), Some(0));
    let report: RedactReportV1 =
        serde_json::from_slice(&json_out.stdout).expect("parse json report");

    assert_eq!(report.events_rewritten_count, events_h);
    assert_eq!(report.objects_redacted_count, objects_h);
}
