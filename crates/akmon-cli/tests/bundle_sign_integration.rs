//! Integration tests for `akmon bundle sign` (D-18 native signing, AGEF v0.1.2).

use std::process::Command;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, generate_pkcs8, parse_public_key_hex,
    read_bundle, verify_manifest_signatures,
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

/// Full produce -> verify loop: export an unsigned bundle, sign it with `akmon bundle sign`,
/// then re-read and verify the signature against the published public key.
#[test]
fn t_bundle_sign_then_signature_verifies() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let session_id = Uuid::new_v4();
    create_clean_session(journal_dir, session_id);

    // 1. Export an unsigned bundle from the journal.
    let bundle = journal_dir.join("session.akmon");
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

    // 2. Generate an Ed25519 PKCS#8 key.
    let key_path = journal_dir.join("signer.pk8");
    std::fs::write(&key_path, generate_pkcs8().expect("keygen")).expect("write key");

    // 3. Sign the bundle in place.
    let sign = akmon()
        .args([
            "bundle",
            "sign",
            &bundle.display().to_string(),
            "--key",
            &key_path.display().to_string(),
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
    let report: Value = serde_json::from_slice(&sign.stdout).expect("sign json");
    let pubkey_hex = report
        .get("public_key_hex")
        .and_then(|v| v.as_str())
        .expect("public_key_hex");
    assert_eq!(
        report.get("signature_count").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report.get("scheme").and_then(Value::as_str),
        Some("ed25519")
    );

    // 4. Re-read the signed bundle and verify the signature with the published public key.
    let mut f = std::fs::File::open(&bundle).expect("open signed bundle");
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = read_bundle(&mut f, &options).expect("read signed bundle");
    assert_eq!(contents.manifest.signatures.as_ref().map(Vec::len), Some(1));
    assert_eq!(contents.manifest.agef_version, "0.1.2");

    let pubkey = parse_public_key_hex(pubkey_hex).expect("parse published pubkey");
    let verification = verify_manifest_signatures(&contents.manifest, &[pubkey]);
    assert!(
        verification.any_verified(),
        "signature should verify with the published public key"
    );
    assert!(!verification.any_invalid());
}

/// Signing with an unreadable/invalid key is a usage error (exit 2), and the bundle is untouched.
#[test]
fn t_bundle_sign_rejects_bad_key() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let session_id = Uuid::new_v4();
    create_clean_session(journal_dir, session_id);

    let bundle = journal_dir.join("session.akmon");
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
    assert!(export.status.success());

    let bad_key = journal_dir.join("bad.pk8");
    std::fs::write(&bad_key, b"not a pkcs8 key").expect("write bad key");

    let sign = akmon()
        .args([
            "bundle",
            "sign",
            &bundle.display().to_string(),
            "--key",
            &bad_key.display().to_string(),
        ])
        .output()
        .expect("run bundle sign");
    assert_eq!(sign.status.code(), Some(2));

    // The bundle remains unsigned (atomic write never ran).
    let mut f = std::fs::File::open(&bundle).expect("open bundle");
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = read_bundle(&mut f, &options).expect("read bundle");
    assert!(contents.manifest.signatures.is_none());
}

/// `akmon bundle verify --verify-key` confirms an akmon-signed bundle (mirrors agef-verify), and
/// `--require-signature` without a key fails.
#[test]
fn t_bundle_verify_with_key_confirms_signature() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let session_id = Uuid::new_v4();
    create_clean_session(journal_dir, session_id);

    let bundle = journal_dir.join("session.akmon");
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
    assert!(export.status.success());

    let key_path = journal_dir.join("signer.pk8");
    std::fs::write(&key_path, generate_pkcs8().expect("keygen")).expect("write key");
    let sign = akmon()
        .args([
            "bundle",
            "sign",
            &bundle.display().to_string(),
            "--key",
            &key_path.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle sign");
    assert!(sign.status.success());
    let report: Value = serde_json::from_slice(&sign.stdout).expect("sign json");
    let pubkey_hex = report
        .get("public_key_hex")
        .and_then(Value::as_str)
        .expect("pubkey");
    let key_file = journal_dir.join("signer.pub.hex");
    std::fs::write(&key_file, pubkey_hex).expect("write pubkey");

    // Verify with the trusted key.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--verify-key",
            &key_file.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle verify");
    assert_eq!(
        verify.status.code(),
        Some(0),
        "stderr={}",
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
        Some("verified")
    );

    // --require-signature without a key fails.
    let req = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--require-signature",
        ])
        .output()
        .expect("run bundle verify require");
    assert_eq!(req.status.code(), Some(1));
}
