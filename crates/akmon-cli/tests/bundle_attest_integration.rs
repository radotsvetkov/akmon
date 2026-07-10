//! Integration tests for `akmon bundle attest` and operator-identity verification flags
//! (decision D-20, AGEF v0.1.3; build 7b).
//!
//! These drive the real `akmon` binary end to end: export a bundle, generate an operator key,
//! attest, and verify with `akmon bundle verify --operator-key`. The honesty contract (O8) is
//! exercised by asserting that `operator_key_verified` is a boolean distinct from the self-asserted
//! `operator_id`, and the O9 agef_version rule is exercised by attesting an already-head-signed
//! bundle and confirming the head signature still verifies.

use std::path::Path;
use std::process::Command;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, WriteBundleOptions, read_bundle, write_bundle,
};
use serde_json::Value;
use tempfile::tempdir;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::*;

fn akmon() -> Command {
    Command::new(akmon_bin_path())
}

/// Exports a clean session as an unsigned bundle at `bundle` and returns nothing.
fn export_bundle(journal_dir: &Path, session_id: Uuid, bundle: &Path) {
    create_clean_session(journal_dir, session_id);
    let export = akmon()
        .current_dir(journal_dir)
        .args([
            "bundle",
            "export",
            &session_id.to_string(),
            "--journal",
            &journal_dir.display().to_string(),
            "--output",
            &bundle.display().to_string(),
        ])
        .output()
        .expect("run bundle export");
    assert!(
        export.status.success(),
        "export stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
}

/// Generates an operator (or signing) key at `key_path` and writes its public hex to `pub_path`.
fn keygen(key_path: &Path, pub_path: &Path) {
    let out = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &key_path.display().to_string(),
            "--public-out",
            &pub_path.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle keygen");
    assert!(
        out.status.success(),
        "keygen stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Attests `bundle` in place with `key`, `operator_id`, and `role`. Returns the parsed JSON report.
fn attest(bundle: &Path, key: &Path, operator_id: &str, role: &str) -> Value {
    let out = akmon()
        .args([
            "bundle",
            "attest",
            &bundle.display().to_string(),
            "--key",
            &key.display().to_string(),
            "--operator-id",
            operator_id,
            "--role",
            role,
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle attest");
    assert!(
        out.status.success(),
        "attest stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("attest json")
}

/// Attest then verify with the operator key + `--require-operator`: the attestation verifies and the
/// self-asserted identity is surfaced verbatim only after key verification.
#[test]
fn t_attest_then_verify_roundtrip() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);

    let report = attest(&bundle, &op_key, "alice@example.com", "approver");
    assert_eq!(
        report.get("operator_id").and_then(Value::as_str),
        Some("alice@example.com")
    );
    // The private key is never surfaced.
    assert!(report.get("private_key").is_none());
    assert!(
        report
            .get("public_key_hex")
            .and_then(Value::as_str)
            .is_some()
    );

    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--operator-key",
            &op_pub.display().to_string(),
            "--require-operator",
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "verify stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(true));
    let ops = v
        .get("operators")
        .and_then(Value::as_array)
        .expect("operators");
    assert_eq!(
        ops[0].get("outcome").and_then(Value::as_str),
        Some("verified")
    );
    assert_eq!(
        ops[0].get("operator_key_verified").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        ops[0].get("operator_id").and_then(Value::as_str),
        Some("alice@example.com")
    );
}

/// `--require-operator` on a bundle with NO attestation fails (exit 1).
#[test]
fn t_require_operator_fails_when_unattested() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--require-operator",
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(verify.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(false));
    assert_eq!(
        v.get("operators").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );
}

/// An attested bundle verified WITHOUT a trusted operator key is `unverified_no_key`, NOT a failure
/// (exit 0). Adding `--require-operator` (still without the key) turns it into a failure (exit 1).
#[test]
fn t_attested_but_no_key_is_unverified_not_failure() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);
    attest(&bundle, &op_key, "alice@example.com", "approver");

    // Verify WITHOUT --operator-key and WITHOUT --require-operator -> exit 0, unverified_no_key.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "verify stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(true));
    let ops = v
        .get("operators")
        .and_then(Value::as_array)
        .expect("operators");
    assert_eq!(
        ops[0].get("outcome").and_then(Value::as_str),
        Some("unverified_no_key")
    );
    assert_eq!(
        ops[0].get("operator_key_verified").and_then(Value::as_bool),
        Some(false)
    );

    // Same, but WITH --require-operator -> exit 1.
    let verify2 = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--require-operator",
        ])
        .output()
        .expect("run bundle verify require");
    assert_eq!(verify2.status.code(), Some(1));
}

/// Attesting with key A then verifying with key B (a different keygen) yields `unverified_no_key`
/// (the key_id does not match), and the bundle still passes without `--require-operator`.
#[test]
fn t_wrong_operator_key_unverified() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let key_a = journal_dir.join("a.pk8");
    let pub_a = journal_dir.join("a.pub.hex");
    keygen(&key_a, &pub_a);
    let key_b = journal_dir.join("b.pk8");
    let pub_b = journal_dir.join("b.pub.hex");
    keygen(&key_b, &pub_b);

    attest(&bundle, &key_a, "alice@example.com", "approver");

    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--operator-key",
            &pub_b.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "verify stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(true));
    let ops = v
        .get("operators")
        .and_then(Value::as_array)
        .expect("operators");
    assert_eq!(
        ops[0].get("outcome").and_then(Value::as_str),
        Some("unverified_no_key")
    );
    assert_eq!(
        ops[0].get("operator_key_verified").and_then(Value::as_bool),
        Some(false)
    );
}

/// Sign a bundle's head, then attest it: the head signature STILL verifies (attest did not rewrite
/// agef_version and break AGEF-SIG-v1, O9) AND the operator attestation verifies.
#[test]
fn t_attest_preserves_head_signature() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    // Generate a SIGNING key and sign the head.
    let signer_key = journal_dir.join("signer.pk8");
    let signer_pub = journal_dir.join("signer.pub.hex");
    keygen(&signer_key, &signer_pub);
    let sign = akmon()
        .args([
            "bundle",
            "sign",
            &bundle.display().to_string(),
            "--key",
            &signer_key.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle sign");
    assert!(
        sign.status.success(),
        "sign stderr={}",
        String::from_utf8_lossy(&sign.stderr)
    );

    // Sanity: the head signature verifies before attesting.
    let pre = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--verify-key",
            &signer_pub.display().to_string(),
            "--require-signature",
            "--format",
            "json",
        ])
        .output()
        .expect("run pre-attest verify");
    assert_eq!(pre.status.code(), Some(0));

    // Now attest with a separate operator key.
    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);
    attest(&bundle, &op_key, "alice@example.com", "approver");

    // The head signature STILL verifies AND the operator verifies.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--verify-key",
            &signer_pub.display().to_string(),
            "--require-signature",
            "--operator-key",
            &op_pub.display().to_string(),
            "--require-operator",
            "--format",
            "json",
        ])
        .output()
        .expect("run post-attest verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "post-attest verify stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(true));
    let sigs = v
        .get("signatures")
        .and_then(Value::as_array)
        .expect("signatures");
    assert_eq!(
        sigs[0].get("outcome").and_then(Value::as_str),
        Some("verified"),
        "head signature must still verify after attest (O9 agef_version rule)"
    );
    let ops = v
        .get("operators")
        .and_then(Value::as_array)
        .expect("operators");
    assert_eq!(
        ops[0].get("outcome").and_then(Value::as_str),
        Some("verified")
    );
}

/// The JSON keeps `operator_key_verified` as a distinct boolean from the self-asserted
/// `operator_id` string (the honesty contract O8): the identity string is reported verbatim, but
/// trust is carried only by `operator_key_verified`.
#[test]
fn t_attest_json_separates_key_verified_from_identity() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);
    attest(&bundle, &op_key, "mallory@evil.example", "ceo");

    // WITHOUT a trusted key: the identity string is present, but operator_key_verified is FALSE.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(verify.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    let op = &v
        .get("operators")
        .and_then(Value::as_array)
        .expect("operators")[0];
    // operator_id is a string field; operator_key_verified is a distinct boolean field.
    assert_eq!(
        op.get("operator_id").and_then(Value::as_str),
        Some("mallory@evil.example")
    );
    assert_eq!(
        op.get("operator_key_verified").and_then(Value::as_bool),
        Some(false),
        "a self-asserted identity must NOT read as key-verified without a trusted key"
    );
    assert!(
        op.get("operator_key_verified").unwrap().is_boolean(),
        "operator_key_verified must be a boolean, not the identity string"
    );
}

/// An operator attestation that names a trusted key by `key_id` but no longer validates (a
/// tampered signature byte) is reported `invalid`, not `unverified_no_key`. Per D-20 an invalid
/// attestation against a trusted key is always a hard failure, even without `--require-operator`.
#[test]
fn t_tampered_operator_attestation_is_invalid() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);
    attest(&bundle, &op_key, "alice@example.com", "approver");

    // Flip a byte in the stored attestation signature, leaving key_id untouched.
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let mut contents =
        read_bundle(std::fs::File::open(&bundle).expect("open"), &options).expect("read bundle");
    let attestations = contents
        .manifest
        .operator_attestations
        .as_mut()
        .expect("attestations");
    let mut sig_bytes = hex::decode(&attestations[0].signature).expect("decode sig");
    sig_bytes[0] ^= 0xFF;
    attestations[0].signature = hex::encode(sig_bytes);
    let mut out = Vec::new();
    write_bundle(
        &mut out,
        &contents.manifest,
        &contents.events,
        &contents.objects,
        &WriteBundleOptions::default(),
    )
    .expect("write bundle");
    std::fs::write(&bundle, out).expect("overwrite bundle");

    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--operator-key",
            &op_pub.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(verify.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(false));
    let ops = v
        .get("operators")
        .and_then(Value::as_array)
        .expect("operators");
    assert_eq!(
        ops[0].get("outcome").and_then(Value::as_str),
        Some("invalid")
    );
}

/// `--require-operator-key <K>` passes when the attestation for exactly that key verified.
#[test]
fn t_require_operator_key_passes_when_that_key_attested() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);
    attest(&bundle, &op_key, "alice@example.com", "approver");

    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--require-operator-key",
            &op_pub.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "verify stderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(true));
}

/// `--require-operator-key <K>` fails (exit 1) when only a DIFFERENT operator key attested: the
/// release-gating policy demands that specific key, not merely any verified attestation.
#[test]
fn t_require_operator_key_fails_when_different_key_attested() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let key_a = journal_dir.join("a.pk8");
    let pub_a = journal_dir.join("a.pub.hex");
    keygen(&key_a, &pub_a);
    let key_b = journal_dir.join("b.pk8");
    let pub_b = journal_dir.join("b.pub.hex");
    keygen(&key_b, &pub_b);

    // Only key A attests; the release policy requires key B specifically.
    attest(&bundle, &key_a, "alice@example.com", "approver");

    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--require-operator-key",
            &pub_b.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(verify.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(v.get("passed").and_then(Value::as_bool), Some(false));
}
