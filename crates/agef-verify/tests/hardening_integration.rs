//! Rejection-path and honesty tests for `agef-verify` (TC-02, TC-03, TC-07, TC-09, TC-10, TC-13,
//! TC-16, TC-17, and the `--format markdown` auditor report).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Command;

use akmon_bundle::manifest::{
    Manifest, ManifestSignature, OperatorAttestation, Producer, SessionMetadata,
};
use akmon_bundle::{
    BundleContents, SCHEME_ED25519, SIG_STATEMENT_VERSION, WriteBundleOptions, generate_pkcs8,
    key_id, public_key_from_pkcs8, sign_statement, signing_statement, write_bundle,
};
use akmon_journal::{AGEF_SPEC_VERSION, Event, EventKind, Hash, HashAlgorithm, digest_bytes};
use serde_json::Value;
use tempfile::tempdir;

fn object(byte: u8) -> (Hash, Vec<u8>) {
    let bytes = vec![byte; 8];
    (digest_bytes(HashAlgorithm::Sha256, &bytes), bytes)
}

fn valid_bundle() -> BundleContents {
    let (cwd_hash, cwd_bytes) = object(0x11);
    let (config_hash, config_bytes) = object(0x12);
    let start = Event {
        parents: vec![],
        kind: EventKind::SessionStart {
            cwd_hash: cwd_hash.clone(),
            config_hash: config_hash.clone(),
        },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("ts"),
        sequence: 0,
    };
    let start_hash = start.content_hash(HashAlgorithm::Sha256).expect("hash");
    let end = Event {
        parents: vec![start_hash],
        kind: EventKind::SessionEnd { summary_hash: None },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_001).expect("ts"),
        sequence: 1,
    };
    let end_hash = end.content_hash(HashAlgorithm::Sha256).expect("hash");
    BundleContents {
        manifest: Manifest {
            agef_version: AGEF_SPEC_VERSION.to_owned(),
            producer: Producer {
                name: "akmon".to_owned(),
                version: "test".to_owned(),
            },
            session: SessionMetadata {
                id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                head: end_hash.to_hex(),
                created_at: "2023-11-14T22:13:20Z".to_owned(),
                ended_at: "2023-11-14T22:13:21Z".to_owned(),
            },
            hash_algorithm: "sha256".to_owned(),
            object_count: 2,
            event_count: 2,
            signatures: None,
            operator_attestations: None,
            extra: BTreeMap::new(),
        },
        events: vec![start, end],
        objects: HashMap::from([(cwd_hash, cwd_bytes), (config_hash, config_bytes)]),
    }
}

fn bundle_bytes(c: &BundleContents) -> Vec<u8> {
    let mut out = Vec::new();
    write_bundle(
        &mut out,
        &c.manifest,
        &c.events,
        &c.objects,
        &WriteBundleOptions::default(),
    )
    .expect("write bundle");
    out
}

fn write_file(path: &Path, c: &BundleContents) {
    std::fs::write(path, bundle_bytes(c)).expect("write file");
}

fn agef_verify() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agef-verify"))
}

/// Signs the bundle head; returns the hex public key.
fn sign(c: &mut BundleContents) -> Vec<u8> {
    let pkcs8 = generate_pkcs8().expect("keygen");
    let pk = public_key_from_pkcs8(&pkcs8).expect("pk");
    let m = &c.manifest;
    let stmt = signing_statement(
        &m.agef_version,
        &m.hash_algorithm,
        &m.session.id,
        &m.session.head,
    );
    let sig = sign_statement(stmt.as_bytes(), &pkcs8).expect("sign");
    c.manifest.signatures = Some(vec![ManifestSignature {
        scheme: SCHEME_ED25519.to_owned(),
        key_id: key_id(&pk),
        statement_version: SIG_STATEMENT_VERSION.to_owned(),
        signature: hex::encode(sig),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
    }]);
    pk
}

fn json_of(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "json: {e}; stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[test]
fn t_headline_unsigned_says_integrity_only() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &valid_bundle());
    let out = agef_verify().arg(&bundle).output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr.lines().next().unwrap_or_default();
    assert!(first.contains("UNSIGNED (integrity only)"), "{stderr}");
    assert!(!first.contains("integrity + signature"), "{stderr}");
}

#[test]
fn t_headline_and_json_name_verified_signer() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle();
    let pk = sign(&mut c);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let pk_file = dir.path().join("pk.hex");
    std::fs::write(&pk_file, hex::encode(&pk)).expect("write pk");

    let out = agef_verify().arg(&bundle).output().expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("signature present but NOT verified"),
        "{stderr}"
    );

    let out = agef_verify()
        .arg(&bundle)
        .arg("--verify-key")
        .arg(&pk_file)
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("signature verified by key_id={}", key_id(&pk))),
        "{stderr}"
    );

    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run");
    let v = json_of(&out);
    assert_eq!(v["passed"], true);
    assert_eq!(v["signature_verified"], false);
    assert_eq!(v["operator_key_verified"], false);
    assert_eq!(v["integrity_ok"], true);
}

#[test]
fn t_injected_operator_fields_cannot_forge_report_lines() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle();
    c.manifest.operator_attestations = Some(vec![OperatorAttestation {
        scheme: "zz\n  - verified by operator key key_id=TRUSTED".to_owned(),
        key_id: "\u{1b}]0;pwned\u{7}".to_owned(),
        statement_version: "AGEF-OPERATOR-v1".to_owned(),
        operator_id: "ceo\u{202e}".to_owned(),
        display_name: String::new(),
        role: String::new(),
        org: String::new(),
        signature: "00".to_owned(),
        created_at: "x".to_owned(),
    }]);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out = agef_verify().arg(&bundle).output().expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr
            .lines()
            .any(|l| l.trim_start().starts_with("- verified by operator key")),
        "{stderr}"
    );
    assert!(
        !stderr.contains('\u{1b}') && !stderr.contains('\u{7}'),
        "{stderr}"
    );
    assert!(!stderr.contains('\u{202e}'), "{stderr}");
}

#[test]
fn t_unreferenced_object_fails() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle();
    let (h, b) = object(0x99);
    c.objects.insert(h, b);
    c.manifest.object_count = 3;
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    let cats: Vec<&str> = v["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .filter_map(|x| x["category"].as_str())
        .collect();
    assert_eq!(cats, vec!["unreferenced_object"], "{v}");
}

#[test]
fn t_metadata_mismatch_is_a_warning_not_a_failure() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle();
    c.manifest.session.ended_at = "2031-01-01T00:00:00Z".to_owned();
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(
        v["warnings"][0]["category"], "manifest_ended_at_mismatch",
        "{v}"
    );
    let human = agef_verify().arg(&bundle).output().expect("run");
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        stderr.contains("unsigned metadata (not covered by any signature)"),
        "{stderr}"
    );
    assert!(stderr.contains("manifest_ended_at_mismatch"), "{stderr}");
}

#[test]
fn t_corrupt_archive_exits_1_with_shared_error_shape() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("b.akmon");
    let mut bytes = bundle_bytes(&valid_bundle());
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&bundle, bytes).expect("write");
    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run");
    assert_eq!(
        out.status.code(),
        Some(1),
        "malformed archive = verification failed"
    );
    let v = json_of(&out);
    assert_eq!(v["tool"], "agef-verify");
    assert_eq!(v["passed"], false);
    assert!(v["akmon_version"].is_string(), "{v}");
    assert!(v["category"].is_string(), "{v}");
}

#[test]
fn t_missing_file_exits_3() {
    let dir = tempdir().expect("tempdir");
    let out = agef_verify()
        .arg(dir.path().join("missing.akmon"))
        .args(["--format", "json"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(3));
    let v = json_of(&out);
    assert_eq!(v["category"], "io_error");
    assert_eq!(v["passed"], false);
}

#[test]
fn t_max_decoded_bytes_flag() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &valid_bundle());
    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "json", "--max-decoded-bytes", "100"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json_of(&out)["category"], "bundle_too_large");
}

#[test]
fn t_markdown_report_for_unsigned_bundle() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &valid_bundle());
    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "markdown"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(
        md.contains("## VERDICT: PASSED — integrity verified (no signature verified)"),
        "{md}"
    );
    assert!(md.contains("| Head signature | NONE |"), "{md}");
    assert!(md.contains("FULL (native Akmon capture)"), "{md}");
    assert!(md.contains("2023-11-14T22:13:20Z"), "{md}");
    assert!(md.contains("Who produced the bundle"), "{md}");
    assert!(md.contains("RFC 3161"), "{md}");
}

#[test]
fn t_markdown_report_for_failed_bundle() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle();
    c.manifest.event_count = 7;
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "markdown"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.contains("## VERDICT: FAILED"), "{md}");
    assert!(md.contains("manifest\\_event\\_count\\_mismatch"), "{md}");
}

/// A valid two-event bundle whose `SessionStart` config object is `config_bytes`.
fn bundle_with_config(config_bytes: &[u8]) -> BundleContents {
    let mut c = valid_bundle();
    let config_hash = digest_bytes(HashAlgorithm::Sha256, config_bytes);
    let EventKind::SessionStart {
        config_hash: old, ..
    } = &mut c.events[0].kind
    else {
        panic!("first event is SessionStart");
    };
    let old_hash = old.clone();
    *old = config_hash.clone();
    c.objects.remove(&old_hash);
    c.objects.insert(config_hash, config_bytes.to_vec());
    let start_hash = c.events[0]
        .content_hash(HashAlgorithm::Sha256)
        .expect("hash");
    c.events[1].parents = vec![start_hash];
    let end_hash = c.events[1]
        .content_hash(HashAlgorithm::Sha256)
        .expect("hash");
    c.manifest.session.head = end_hash.to_hex();
    c
}

#[test]
fn t_partial_capture_is_surfaced_honestly() {
    let dir = tempdir().expect("tempdir");
    let c = bundle_with_config(
        br#"{"akmon_otel_config":true,"capture_level":"partial","source_semconv":"1.37.0"}"#,
    );
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);

    let out = agef_verify().arg(&bundle).output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr.lines().next().unwrap_or_default();
    assert!(first.contains("capture: PARTIAL"), "{stderr}");
    assert!(stderr.contains("NOT full capture"), "{stderr}");

    let out = agef_verify()
        .arg(&bundle)
        .args(["--format", "markdown"])
        .output()
        .expect("run");
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.contains("PARTIAL (OTEL import)"), "{md}");

    let out = agef_verify()
        .arg(&bundle)
        .args(["--require-capture", "full", "--format", "json"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["capture"]["level"], "partial");
    let msg = v["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .find(|x| x["category"] == "capture_requirement_unmet")
        .expect("capture violation")["message"]
        .as_str()
        .expect("message")
        .to_owned();
    assert!(msg.contains("partial OTEL import"), "{msg}");
    assert!(!msg.contains("metadata-only"), "{msg}");
}

#[test]
fn t_signature_flag_is_false_when_integrity_fails() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle();
    let pk = sign(&mut c);
    // event_count is not covered by the signed statement: the signature still verifies, but
    // integrity fails.
    c.manifest.event_count = 7;
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let pk_file = dir.path().join("pk.hex");
    std::fs::write(&pk_file, hex::encode(&pk)).expect("write pk");
    let out = agef_verify()
        .arg(&bundle)
        .arg("--verify-key")
        .arg(&pk_file)
        .args(["--format", "json"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    let v = json_of(&out);
    assert_eq!(v["integrity_ok"], false);
    assert_eq!(v["passed"], false);
    assert_eq!(v["signature_verified"], false);
    assert_eq!(v["operator_key_verified"], false);
    assert!(
        v["signatures"]
            .as_array()
            .expect("signatures")
            .iter()
            .any(|s| s["outcome"] == "verified"),
        "{v}"
    );
}
