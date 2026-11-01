//! Rejection-path and honesty tests for `akmon bundle sign` / `attest` / `prove-openssl` /
//! `import` / `verify` (TC-01, TC-02, TC-03, TC-04, TC-05, TC-09, TC-11, TC-13, TC-17).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use akmon_bundle::manifest::{Manifest, ManifestSignature, Producer, SessionMetadata};
use akmon_bundle::{
    BundleContents, OperatorIdentity, ReadBundleOptions, SCHEME_ED25519, SIG_STATEMENT_VERSION,
    WriteBundleOptions, build_operator_attestation, generate_pkcs8, key_id, public_key_from_pkcs8,
    read_bundle, sign_statement, signing_statement, verify_manifest_signatures,
    verify_operator_attestations, write_bundle,
};
use akmon_journal::{Event, EventKind, Hash, HashAlgorithm, digest_bytes};
use serde_json::Value;
use tempfile::tempdir;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::{akmon_bin_path, create_clean_session};

fn akmon() -> Command {
    Command::new(akmon_bin_path())
}

fn object(algo: HashAlgorithm, byte: u8) -> (Hash, Vec<u8>) {
    let bytes = vec![byte; 8];
    (digest_bytes(algo, &bytes), bytes)
}

/// A valid two-event bundle with the given algorithm and AGEF version.
fn valid_bundle(algo: HashAlgorithm, agef_version: &str) -> BundleContents {
    let (cwd_hash, cwd_bytes) = object(algo, 0x11);
    let (config_hash, config_bytes) = object(algo, 0x12);
    let start = Event {
        parents: vec![],
        kind: EventKind::SessionStart {
            cwd_hash: cwd_hash.clone(),
            config_hash: config_hash.clone(),
        },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("ts"),
        sequence: 0,
    };
    let start_hash = start.content_hash(algo).expect("hash");
    let end = Event {
        parents: vec![start_hash],
        kind: EventKind::SessionEnd { summary_hash: None },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_001).expect("ts"),
        sequence: 1,
    };
    let end_hash = end.content_hash(algo).expect("hash");
    BundleContents {
        manifest: Manifest {
            agef_version: agef_version.to_owned(),
            producer: Producer {
                name: "akmon".to_owned(),
                version: "test".to_owned(),
            },
            session: SessionMetadata {
                id: Uuid::new_v4().as_hyphenated().to_string(),
                head: end_hash.to_hex(),
                created_at: "2023-11-14T22:13:20Z".to_owned(),
                ended_at: "2023-11-14T22:13:21Z".to_owned(),
            },
            hash_algorithm: algo.to_string(),
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

fn write_file(path: &Path, c: &BundleContents) {
    let mut out = Vec::new();
    write_bundle(
        &mut out,
        &c.manifest,
        &c.events,
        &c.objects,
        &WriteBundleOptions::default(),
    )
    .expect("write bundle");
    std::fs::write(path, out).expect("write file");
}

fn read_file(path: &Path) -> BundleContents {
    let file = std::fs::File::open(path).expect("open");
    read_bundle(file, &ReadBundleOptions::default()).expect("read")
}

/// Adds a valid AGEF-SIG-v1 signature by `pkcs8` over the manifest as it currently stands.
fn add_signature(c: &mut BundleContents, pkcs8: &[u8]) {
    let m = &c.manifest;
    let stmt = signing_statement(
        &m.agef_version,
        &m.hash_algorithm,
        &m.session.id,
        &m.session.head,
    );
    let sig = sign_statement(stmt.as_bytes(), pkcs8).expect("sign");
    let pk = public_key_from_pkcs8(pkcs8).expect("pk");
    c.manifest
        .signatures
        .get_or_insert_with(Vec::new)
        .push(ManifestSignature {
            scheme: SCHEME_ED25519.to_owned(),
            key_id: key_id(&pk),
            statement_version: SIG_STATEMENT_VERSION.to_owned(),
            signature: hex::encode(sig),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        });
}

/// Writes a key pair into `dir`: returns (pkcs8 path, public-key-hex path, pkcs8 bytes, pubkey).
fn keypair(dir: &Path, name: &str) -> (PathBuf, PathBuf, Vec<u8>, Vec<u8>) {
    let pkcs8 = generate_pkcs8().expect("keygen");
    let pk = public_key_from_pkcs8(&pkcs8).expect("pk");
    let sk_path = dir.join(format!("{name}.pk8"));
    let pk_path = dir.join(format!("{name}.pub.hex"));
    std::fs::write(&sk_path, &pkcs8).expect("write sk");
    std::fs::write(&pk_path, hex::encode(&pk)).expect("write pk");
    (sk_path, pk_path, pkcs8, pk)
}

/// Breaks the head commitment without touching the manifest: the last event's timestamp changes,
/// so its content hash no longer equals `manifest.session.head`.
fn tamper_events(c: &mut BundleContents) {
    c.events[1].emitted_at = time::OffsetDateTime::from_unix_timestamp(1_700_000_999).expect("ts");
}

fn run_json(args: &[&str]) -> (Option<i32>, Value) {
    let out = akmon().args(args).output().expect("run akmon");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "json: {e}; stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code(), v)
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

#[test]
fn t_prove_openssl_refuses_bundle_whose_head_does_not_match_events() {
    let dir = tempdir().expect("tempdir");
    let (_, pk_path, pkcs8, _) = keypair(dir.path(), "signer");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    add_signature(&mut c, &pkcs8); // valid signature over the (honest) head ...
    tamper_events(&mut c); // ... but the content no longer hashes to it.
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out_dir = dir.path().join("proof");
    let (code, v) = run_json(&[
        "bundle",
        "prove-openssl",
        &s(&bundle),
        "--verify-key",
        &s(&pk_path),
        "--out-dir",
        &s(&out_dir),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(1), "body={v}");
    assert_eq!(v["category"], "integrity_failed");
    assert_eq!(v["passed"], false);
    assert!(
        !out_dir.join("statement.bin").exists(),
        "no artifact may be written"
    );
}

#[test]
fn t_prove_openssl_refuses_signature_that_does_not_verify() {
    let dir = tempdir().expect("tempdir");
    let (_, pk_path, pkcs8, _) = keypair(dir.path(), "signer");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    add_signature(&mut c, &pkcs8);
    // Flip one signature byte: still 64 bytes of hex under the right key_id, but invalid.
    if let Some(sig) = c.manifest.signatures.as_mut().and_then(|v| v.first_mut()) {
        let flipped = if sig.signature.starts_with('0') {
            "1"
        } else {
            "0"
        };
        sig.signature.replace_range(0..1, flipped);
    }
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out_dir = dir.path().join("proof");
    let (code, v) = run_json(&[
        "bundle",
        "prove-openssl",
        &s(&bundle),
        "--verify-key",
        &s(&pk_path),
        "--out-dir",
        &s(&out_dir),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(1), "body={v}");
    assert_eq!(v["category"], "signature_invalid");
    assert!(!out_dir.exists() || std::fs::read_dir(&out_dir).expect("dir").next().is_none());
}

#[test]
fn t_prove_openssl_refuses_invalid_operator_attestation_before_writing_anything() {
    let dir = tempdir().expect("tempdir");
    let (_, pk_path, pkcs8, _) = keypair(dir.path(), "signer");
    let (_, op_pk_path, op_pkcs8, _) = keypair(dir.path(), "operator");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    add_signature(&mut c, &pkcs8);
    let identity = OperatorIdentity {
        operator_id: "alice".to_owned(),
        ..OperatorIdentity::default()
    };
    let mut att =
        build_operator_attestation(&c.manifest, &identity, &op_pkcs8, "2026-01-01T00:00:00Z")
            .expect("attest");
    att.operator_id = "mallory".to_owned(); // identity tampered after signing
    c.manifest.operator_attestations = Some(vec![att]);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out_dir = dir.path().join("proof");
    let (code, v) = run_json(&[
        "bundle",
        "prove-openssl",
        &s(&bundle),
        "--verify-key",
        &s(&pk_path),
        "--operator-key",
        &s(&op_pk_path),
        "--out-dir",
        &s(&out_dir),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(1), "body={v}");
    assert_eq!(v["category"], "operator_signature_invalid");
    assert!(
        !out_dir.join("statement.bin").exists(),
        "head artifacts must not be written when the operator proof is refused"
    );
}

#[test]
fn t_prove_openssl_reports_honest_claims_and_escapes_operator_fields() {
    let dir = tempdir().expect("tempdir");
    let (_, pk_path, pkcs8, _) = keypair(dir.path(), "signer");
    let (_, op_pk_path, op_pkcs8, _) = keypair(dir.path(), "operator");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    add_signature(&mut c, &pkcs8);
    let identity = OperatorIdentity {
        operator_id: "alice\u{1b}[2J\u{202e}".to_owned(),
        role: "approver\u{9b}31m".to_owned(),
        ..OperatorIdentity::default()
    };
    let att = build_operator_attestation(&c.manifest, &identity, &op_pkcs8, "x").expect("att");
    c.manifest.operator_attestations = Some(vec![att]);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out_dir = dir.path().join("proof");
    let base = [
        "bundle".to_owned(),
        "prove-openssl".to_owned(),
        s(&bundle),
        "--verify-key".to_owned(),
        s(&pk_path),
        "--operator-key".to_owned(),
        s(&op_pk_path),
        "--out-dir".to_owned(),
        s(&out_dir),
    ];
    let human = akmon().args(&base).output().expect("run");
    assert_eq!(human.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        !stderr.contains('\u{1b}') && !stderr.contains('\u{202e}') && !stderr.contains('\u{9b}')
    );
    assert!(stderr.contains("alice\\x1b[2J\\u{202e}"), "stderr={stderr}");
    assert!(stderr.contains("signed session head H"), "stderr={stderr}");
    assert!(stderr.contains("agef-verify"), "stderr={stderr}");

    let mut json_args: Vec<&str> = base.iter().map(String::as_str).collect();
    json_args.extend(["--format", "json"]);
    let (code, v) = run_json(&json_args);
    assert_eq!(code, Some(0), "body={v}");
    assert_eq!(v["integrity_verified"], true);
    assert_eq!(v["signature_verified"], true);
    assert_eq!(v["head"], c.manifest.session.head.as_str());
    assert!(
        v["openssl_proves"]
            .as_str()
            .is_some_and(|t| t.contains("head H"))
    );
    assert_eq!(v["operator"]["operator_key_verified"], true);
}

#[test]
fn t_sign_refuses_bundle_that_fails_integrity() {
    let dir = tempdir().expect("tempdir");
    let (sk_path, _, _, _) = keypair(dir.path(), "signer");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    tamper_events(&mut c);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let before = std::fs::read(&bundle).expect("read");
    let (code, v) = run_json(&[
        "bundle",
        "sign",
        &s(&bundle),
        "--key",
        &s(&sk_path),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(1), "body={v}");
    assert_eq!(v["category"], "integrity_failed");
    assert_eq!(v["passed"], false);
    assert_eq!(
        std::fs::read(&bundle).expect("read"),
        before,
        "bundle untouched"
    );
}

#[test]
fn t_attest_refuses_bundle_that_fails_integrity() {
    let dir = tempdir().expect("tempdir");
    let (sk_path, _, _, _) = keypair(dir.path(), "operator");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    tamper_events(&mut c);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let (code, v) = run_json(&[
        "bundle",
        "attest",
        &s(&bundle),
        "--key",
        &s(&sk_path),
        "--operator-id",
        "alice",
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(1), "body={v}");
    assert_eq!(v["category"], "integrity_failed");
}

#[test]
fn t_cosign_preserves_older_agef_version_and_both_signatures_verify() {
    let dir = tempdir().expect("tempdir");
    let (_, _, pkcs8_a, pk_a) = keypair(dir.path(), "a");
    let (sk_b, _, _, pk_b) = keypair(dir.path(), "b");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.2");
    add_signature(&mut c, &pkcs8_a);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let (code, v) = run_json(&[
        "bundle",
        "sign",
        &s(&bundle),
        "--key",
        &s(&sk_b),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(0), "body={v}");
    let signed = read_file(&bundle);
    assert_eq!(
        signed.manifest.agef_version, "0.1.2",
        "co-sign must not restamp"
    );
    let report = verify_manifest_signatures(&signed.manifest, &[pk_a, pk_b]);
    assert_eq!(report.checks.len(), 2);
    assert!(
        report
            .checks
            .iter()
            .all(|c| c.outcome == akmon_bundle::SignatureOutcome::Verified),
        "{report:?}"
    );
}

#[test]
fn t_sign_after_attestation_keeps_attestation_valid() {
    let dir = tempdir().expect("tempdir");
    let (_, _, op_pkcs8, op_pk) = keypair(dir.path(), "operator");
    let (sk, _, _, _) = keypair(dir.path(), "signer");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.2");
    let identity = OperatorIdentity {
        operator_id: "alice".to_owned(),
        ..OperatorIdentity::default()
    };
    let att = build_operator_attestation(&c.manifest, &identity, &op_pkcs8, "x").expect("att");
    c.manifest.operator_attestations = Some(vec![att]);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let (code, v) = run_json(&[
        "bundle",
        "sign",
        &s(&bundle),
        "--key",
        &s(&sk),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(0), "body={v}");
    let signed = read_file(&bundle);
    assert_eq!(signed.manifest.agef_version, "0.1.2");
    let report = verify_operator_attestations(&signed.manifest, &[op_pk]);
    assert!(report.any_verified(), "{report:?}");
}

#[test]
fn t_attest_after_attestation_keeps_first_attestation_valid() {
    let dir = tempdir().expect("tempdir");
    let (_, _, op1_pkcs8, op1_pk) = keypair(dir.path(), "op1");
    let (sk2, _, _, _) = keypair(dir.path(), "op2");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.2");
    let identity = OperatorIdentity {
        operator_id: "alice".to_owned(),
        ..OperatorIdentity::default()
    };
    let att = build_operator_attestation(&c.manifest, &identity, &op1_pkcs8, "x").expect("att");
    c.manifest.operator_attestations = Some(vec![att]);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let (code, v) = run_json(&[
        "bundle",
        "attest",
        &s(&bundle),
        "--key",
        &s(&sk2),
        "--operator-id",
        "bob",
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(0), "body={v}");
    let attested = read_file(&bundle);
    assert_eq!(attested.manifest.agef_version, "0.1.2");
    assert!(verify_operator_attestations(&attested.manifest, &[op1_pk]).any_verified());
}

#[test]
fn t_verify_headline_and_json_flags_reflect_what_was_established() {
    let dir = tempdir().expect("tempdir");
    let (_, pk_path, pkcs8, pk) = keypair(dir.path(), "signer");
    let unsigned = dir.path().join("unsigned.akmon");
    write_file(&unsigned, &valid_bundle(HashAlgorithm::Sha256, "0.1.3"));
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    add_signature(&mut c, &pkcs8);
    let signed = dir.path().join("signed.akmon");
    write_file(&signed, &c);

    let stderr_of = |args: &[&str]| {
        let out = akmon().args(args).output().expect("run");
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let e = stderr_of(&["bundle", "verify", &s(&unsigned)]);
    assert!(e.contains("UNSIGNED (integrity only)"), "stderr={e}");
    let e = stderr_of(&["bundle", "verify", &s(&signed)]);
    assert!(
        e.contains("signature present but NOT verified"),
        "stderr={e}"
    );
    let e = stderr_of(&[
        "bundle",
        "verify",
        &s(&signed),
        "--verify-key",
        &s(&pk_path),
    ]);
    assert!(
        e.contains(&format!("signature verified by key_id={}", key_id(&pk))),
        "stderr={e}"
    );

    let (_, v) = run_json(&["bundle", "verify", &s(&signed), "--format", "json"]);
    assert_eq!(v["passed"], true);
    assert_eq!(v["signature_verified"], false);
    assert_eq!(v["operator_key_verified"], false);
    let (_, v) = run_json(&[
        "bundle",
        "verify",
        &s(&signed),
        "--verify-key",
        &s(&pk_path),
        "--format",
        "json",
    ]);
    assert_eq!(v["signature_verified"], true);
    assert_eq!(v["integrity_ok"], true);
    assert_eq!(v["head"], c.manifest.session.head.as_str());
    assert_eq!(
        v["session_window"]["first_event_at"],
        "2023-11-14T22:13:20Z"
    );
    assert_eq!(v["unsigned_metadata"]["producer_name"], "akmon");
}

#[test]
fn t_verify_escapes_injected_signature_scheme_and_key_id() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    c.manifest.signatures = Some(vec![ManifestSignature {
        scheme: "x] key_id=a\n    - verified [ed25519".to_owned(),
        key_id: "\u{1b}[32mTRUSTED\u{9b}0m".to_owned(),
        statement_version: SIG_STATEMENT_VERSION.to_owned(),
        signature: "00".to_owned(),
        created_at: "x".to_owned(),
    }]);
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out = akmon()
        .args(["bundle", "verify"])
        .arg(&bundle)
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr
            .lines()
            .any(|l| l.trim_start().starts_with("- verified")),
        "a forged 'verified' line must not appear: {stderr}"
    );
    assert!(
        !stderr.contains('\u{1b}') && !stderr.contains('\u{9b}'),
        "{stderr}"
    );
    assert!(stderr.contains("\\x0a"), "{stderr}");
}

#[test]
fn t_verify_warns_when_unsigned_manifest_times_disagree_with_events() {
    let dir = tempdir().expect("tempdir");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    c.manifest.session.created_at = "2030-01-01T00:00:00Z".to_owned();
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let (code, v) = run_json(&["bundle", "verify", &s(&bundle), "--format", "json"]);
    assert_eq!(
        code,
        Some(0),
        "a metadata mismatch is a warning, not a failure: {v}"
    );
    let warnings = v["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 1, "{v}");
    assert_eq!(warnings[0]["category"], "manifest_created_at_mismatch");
    assert_eq!(v["unsigned_metadata"]["created_at"], "2030-01-01T00:00:00Z");
}

#[test]
fn t_verify_markdown_report_is_plain_language_and_escaped() {
    let dir = tempdir().expect("tempdir");
    let (_, pk_path, pkcs8, pk) = keypair(dir.path(), "signer");
    let mut c = valid_bundle(HashAlgorithm::Sha256, "0.1.3");
    add_signature(&mut c, &pkcs8);
    c.manifest.producer.name = "evil | <script>".to_owned();
    let bundle = dir.path().join("b.akmon");
    write_file(&bundle, &c);
    let out = akmon()
        .args(["bundle", "verify"])
        .arg(&bundle)
        .args(["--verify-key", &s(&pk_path), "--format", "markdown"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.contains("## VERDICT: PASSED"), "{md}");
    assert!(md.contains(&key_id(&pk)), "{md}");
    assert!(md.contains("What this does not prove"), "{md}");
    assert!(md.contains("RFC 3161"), "{md}");
    assert!(md.contains("evil \\| \\<script\\>"), "{md}");
    assert!(!md.contains("<script>"), "{md}");
}

#[test]
fn t_import_refuses_hash_algorithm_mismatch_before_writing() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path().join("journal");
    create_clean_session(&journal_dir, Uuid::new_v4()); // sha256 store
    let bundle = dir.path().join("blake3.akmon");
    write_file(&bundle, &valid_bundle(HashAlgorithm::Blake3, "0.1.3"));
    let (code, v) = run_json(&[
        "bundle",
        "import",
        &s(&bundle),
        "--journal",
        &s(&journal_dir),
        "--format",
        "json",
    ]);
    assert_eq!(code, Some(2), "body={v}");
    assert_eq!(v["category"], "hash_algorithm_mismatch");
    assert_eq!(v["passed"], false);
}
