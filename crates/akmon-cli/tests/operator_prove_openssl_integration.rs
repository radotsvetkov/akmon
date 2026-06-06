//! Reproducible-proof integration test for `akmon bundle prove-openssl --operator-key` (decision
//! D-20, build 7c).
//!
//! `prove-openssl --operator-key` emits the operator-attestation artifacts a third party needs to
//! verify an Akmon `AGEF-OPERATOR-v1` operator attestation with stock `openssl` alone (no Akmon
//! binary). This test attests a real bundle, emits the operator artifacts, and — only when a
//! genuinely Ed25519-capable `openssl` is present — runs the real openssl verify (positive) and a
//! tampered-statement case (negative).
//!
//! It SKIPS gracefully (eprintln a note + `return`, never asserts/panics) when no capable openssl
//! is available. This is REQUIRED: the default macOS `/usr/bin/openssl` is LibreSSL, which lacks
//! `-rawin` and cannot load Ed25519 keys, so a presence-only gate would turn CI RED. Point at a
//! capable binary with `AKMON_OPENSSL=<path>`; otherwise the test probes `openssl` on PATH.

use std::path::Path;
use std::process::Command;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, ed25519_spki_pem, generate_pkcs8,
    operator_statement, parse_public_key_hex, read_bundle,
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

/// The openssl binary to probe/use: `AKMON_OPENSSL` override, else `openssl` on PATH.
fn openssl_bin() -> String {
    std::env::var("AKMON_OPENSSL").unwrap_or_else(|_| "openssl".to_owned())
}

/// True when stderr/stdout of a failed openssl call looks like an unsupported-Ed25519 condition.
fn looks_unsupported(out: &std::process::Output) -> bool {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    text.contains("unsupported")
        || text.contains("unable to load")
        || text.contains("not found")
        || text.contains("usage:")
        || text.contains("usage error")
}

/// Probes whether `openssl` can genuinely verify Ed25519 with `-rawin`.
///
/// Covers BOTH required legs: (a) `genpkey -algorithm ed25519` exits 0, and (b) a trial
/// `pkeyutl -verify ... -rawin` against a known-good emitted fixture exits 0. Returns false (SKIP)
/// for binary-not-found, any non-zero exit, or unsupported/usage text — never panics.
fn openssl_can_verify_ed25519(probe_dir: &Path) -> bool {
    let bin = openssl_bin();

    // Leg (a): genpkey -algorithm ed25519 must produce a key (exit 0, no usage text).
    let sk = probe_dir.join("probe_sk.pem");
    let genpkey = Command::new(&bin)
        .args(["genpkey", "-algorithm", "ed25519", "-out"])
        .arg(&sk)
        .output();
    let genpkey = match genpkey {
        Ok(o) => o,
        Err(_) => return false, // binary not found
    };
    if !genpkey.status.success() || looks_unsupported(&genpkey) || !sk.is_file() {
        return false;
    }

    // Leg (b): a real round-trip. Sign a known message with ring, emit the SPKI PEM via our own
    // library primitive, and confirm openssl pkeyutl -verify -rawin returns exit 0. This proves
    // the exact code path the command relies on actually works on this openssl.
    let pkcs8 = match generate_pkcs8() {
        Ok(k) => k,
        Err(_) => return false,
    };
    let pubkey = match akmon_bundle::public_key_from_pkcs8(&pkcs8) {
        Ok(pk) => pk,
        Err(_) => return false,
    };
    let msg = b"akmon-openssl-probe";
    let sig = match akmon_bundle::sign_statement(msg, &pkcs8) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pem = match ed25519_spki_pem(&pubkey) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let pem_path = probe_dir.join("probe_pub.pem");
    let msg_path = probe_dir.join("probe_msg.bin");
    let sig_path = probe_dir.join("probe_sig.bin");
    if std::fs::write(&pem_path, pem.as_bytes()).is_err()
        || std::fs::write(&msg_path, msg).is_err()
        || std::fs::write(&sig_path, &sig).is_err()
    {
        return false;
    }
    let verify = Command::new(&bin)
        .args(["pkeyutl", "-verify", "-pubin", "-inkey"])
        .arg(&pem_path)
        .arg("-rawin")
        .arg("-in")
        .arg(&msg_path)
        .arg("-sigfile")
        .arg(&sig_path)
        .output();
    match verify {
        Ok(o) => o.status.success() && !looks_unsupported(&o),
        Err(_) => false,
    }
}

/// Exports a clean session as an unsigned bundle at `bundle`.
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

/// Generates an Ed25519 key at `key_path` and writes its public key hex to `pub_path`.
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

/// Signs `bundle`'s head in place with `key`.
fn sign(bundle: &Path, key: &Path) {
    let out = akmon()
        .args([
            "bundle",
            "sign",
            &bundle.display().to_string(),
            "--key",
            &key.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle sign");
    assert!(
        out.status.success(),
        "sign stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Attests `bundle` in place with `key`, `operator_id`, and `role`.
fn attest(bundle: &Path, key: &Path, operator_id: &str, role: &str) {
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
}

/// Builds a bundle that is BOTH head-signed and operator-attested, returning
/// (bundle_path, signer_pub_hex_file, operator_pub_hex_file).
///
/// `prove-openssl` requires `--verify-key` (the head signature), so the bundle is signed with a
/// distinct signing key whose public half is returned for `--verify-key`, and attested with a
/// separate operator key whose public half is returned for `--operator-key`.
fn build_signed_and_attested_bundle(
    journal_dir: &Path,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let bundle = journal_dir.join("session.akmon");
    export_bundle(journal_dir, Uuid::new_v4(), &bundle);

    let signer_key = journal_dir.join("signer.pk8");
    let signer_pub = journal_dir.join("signer.pub.hex");
    keygen(&signer_key, &signer_pub);
    sign(&bundle, &signer_key);

    let op_key = journal_dir.join("operator.pk8");
    let op_pub = journal_dir.join("operator.pub.hex");
    keygen(&op_key, &op_pub);
    attest(&bundle, &op_key, "ops@example.com", "approver");

    (bundle, signer_pub, op_pub)
}

/// Runs `bundle prove-openssl` with both `--verify-key` and `--operator-key`, returning the parsed
/// JSON report.
fn prove_with_operator(
    bundle: &Path,
    verify_key: &Path,
    operator_key: &Path,
    out_dir: &Path,
) -> Value {
    let prove = akmon()
        .args([
            "bundle",
            "prove-openssl",
            &bundle.display().to_string(),
            "--verify-key",
            &verify_key.display().to_string(),
            "--operator-key",
            &operator_key.display().to_string(),
            "--out-dir",
            &out_dir.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle prove-openssl");
    assert_eq!(
        prove.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&prove.stderr)
    );
    serde_json::from_slice(&prove.stdout).expect("prove json")
}

/// The reproducible proof: a third party with ONLY openssl verifies the operator attestation
/// (positive), and a one-byte tamper of the operator statement makes openssl fail (negative).
/// Skips when openssl cannot do Ed25519 (LibreSSL CI default) — never fails CI in that case.
#[test]
fn t_operator_prove_openssl_verifies_and_tamper_fails() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();

    let probe_dir = journal_dir.join("probe");
    std::fs::create_dir_all(&probe_dir).expect("mkdir probe");
    if !openssl_can_verify_ed25519(&probe_dir) {
        eprintln!(
            "SKIP t_operator_prove_openssl_verifies_and_tamper_fails: no Ed25519-capable openssl \
             found (probed `{}`). Set AKMON_OPENSSL=<OpenSSL 3.x path> to run the real proof. \
             The deterministic artifact test still covered byte-identity.",
            openssl_bin()
        );
        return;
    }

    let (bundle, signer_pub, op_pub) = build_signed_and_attested_bundle(journal_dir);
    let out_dir = journal_dir.join("proof");
    let report = prove_with_operator(&bundle, &signer_pub, &op_pub, &out_dir);

    let operator = report.get("operator").expect("operator block present");
    let openssl_command = operator
        .get("openssl_command")
        .and_then(Value::as_str)
        .expect("operator openssl_command");
    // The printed command names the standard verify invocation (used verbatim below).
    assert!(openssl_command.contains("pkeyutl -verify -pubin -inkey"));
    assert!(openssl_command.contains("-rawin"));

    let bin = openssl_bin();
    let statement_path = out_dir.join("operator_statement.bin");
    let signature_path = out_dir.join("operator_signature.bin");
    let pubkey_pem_path = out_dir.join("operator_pubkey.pem");

    // POSITIVE: openssl verifies the emitted operator artifacts (exit 0).
    let verify = Command::new(&bin)
        .args(["pkeyutl", "-verify", "-pubin", "-inkey"])
        .arg(&pubkey_pem_path)
        .arg("-rawin")
        .arg("-in")
        .arg(&statement_path)
        .arg("-sigfile")
        .arg(&signature_path)
        .output()
        .expect("run openssl verify");
    assert!(
        verify.status.success(),
        "openssl should verify the emitted operator artifacts; stdout={} stderr={}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );

    // NEGATIVE: flip one byte of operator_statement.bin -> openssl verification must fail.
    let mut tampered = std::fs::read(&statement_path).expect("read operator statement");
    let last = tampered.last_mut().expect("non-empty statement");
    *last ^= 0x01;
    let tampered_path = out_dir.join("operator_statement.tampered.bin");
    std::fs::write(&tampered_path, &tampered).expect("write tampered statement");
    let verify_bad = Command::new(&bin)
        .args(["pkeyutl", "-verify", "-pubin", "-inkey"])
        .arg(&pubkey_pem_path)
        .arg("-rawin")
        .arg("-in")
        .arg(&tampered_path)
        .arg("-sigfile")
        .arg(&signature_path)
        .output()
        .expect("run openssl verify tampered");
    assert!(
        !verify_bad.status.success(),
        "openssl must reject a tampered operator statement; stdout={} stderr={}",
        String::from_utf8_lossy(&verify_bad.stdout),
        String::from_utf8_lossy(&verify_bad.stderr)
    );
}

/// Deterministic, no-openssl: the emitted operator_statement.bin is byte-identical to the library
/// `operator_statement(...)` reconstructed from the read-back manifest + the matching attestation,
/// operator_signature.bin is 64 bytes, and operator_pubkey.pem is the SPKI PEM.
#[test]
fn t_operator_prove_artifacts_byte_identical() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let (bundle, signer_pub, op_pub) = build_signed_and_attested_bundle(journal_dir);

    let out_dir = journal_dir.join("proof");
    let report = prove_with_operator(&bundle, &signer_pub, &op_pub, &out_dir);
    assert!(report.get("operator").is_some(), "operator block present");

    let statement_path = out_dir.join("operator_statement.bin");
    let signature_path = out_dir.join("operator_signature.bin");
    let pubkey_pem_path = out_dir.join("operator_pubkey.pem");

    // Read back the manifest and the matching operator attestation entry.
    let mut f = std::fs::File::open(&bundle).expect("open attested bundle");
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = read_bundle(&mut f, &options).expect("read attested bundle");
    let m = &contents.manifest;
    let op_pub_hex = std::fs::read_to_string(&op_pub).expect("read operator pubkey hex");
    let operator_pubkey = parse_public_key_hex(&op_pub_hex).expect("parse operator pubkey");
    let expected_kid = akmon_bundle::key_id(&operator_pubkey);
    let entry = m
        .operator_attestations
        .as_ref()
        .expect("operator_attestations present")
        .iter()
        .find(|a| a.key_id == expected_kid)
        .expect("matching operator attestation");

    // (1) operator_statement.bin == library operator_statement(...) over the manifest + entry.
    let expected_statement = operator_statement(
        &m.agef_version,
        &m.hash_algorithm,
        &m.session.id,
        &m.session.head,
        &entry.operator_id,
        &entry.display_name,
        &entry.role,
        &entry.org,
    );
    let statement_bytes = std::fs::read(&statement_path).expect("read operator_statement.bin");
    assert_eq!(
        statement_bytes,
        expected_statement.as_bytes(),
        "operator_statement.bin must be byte-identical to operator_statement(...)"
    );

    // (2) operator_signature.bin is exactly 64 bytes.
    let signature_bytes = std::fs::read(&signature_path).expect("read operator_signature.bin");
    assert_eq!(
        signature_bytes.len(),
        64,
        "raw Ed25519 operator signature is 64 bytes"
    );

    // (3) operator_pubkey.pem begins/ends with the SPKI PEM guard lines and equals the library PEM.
    let pem_bytes = std::fs::read_to_string(&pubkey_pem_path).expect("read operator_pubkey.pem");
    assert_eq!(
        pem_bytes,
        ed25519_spki_pem(&operator_pubkey).expect("pem"),
        "operator_pubkey.pem must equal the library SPKI PEM"
    );
    assert!(pem_bytes.starts_with("-----BEGIN PUBLIC KEY-----\n"));
    assert!(pem_bytes.ends_with("-----END PUBLIC KEY-----\n"));
}

/// Additive/no-regression: WITHOUT `--operator-key`, the JSON has NO `operator` key and the three
/// operator_* files are absent.
#[test]
fn t_prove_without_operator_key_unchanged() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let (bundle, signer_pub, _op_pub) = build_signed_and_attested_bundle(journal_dir);

    let out_dir = journal_dir.join("proof");
    let prove = akmon()
        .args([
            "bundle",
            "prove-openssl",
            &bundle.display().to_string(),
            "--verify-key",
            &signer_pub.display().to_string(),
            "--out-dir",
            &out_dir.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle prove-openssl");
    assert_eq!(
        prove.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&prove.stderr)
    );
    let report: Value = serde_json::from_slice(&prove.stdout).expect("prove json");

    // The "operator" key must be entirely absent (serde skip_serializing_if).
    assert!(
        report.get("operator").is_none(),
        "without --operator-key the JSON must have no `operator` key"
    );
    // The head-signature artifacts are present; the operator_* artifacts are absent.
    assert!(out_dir.join("statement.bin").is_file());
    assert!(out_dir.join("signature.bin").is_file());
    assert!(out_dir.join("pubkey.pem").is_file());
    assert!(!out_dir.join("operator_statement.bin").exists());
    assert!(!out_dir.join("operator_signature.bin").exists());
    assert!(!out_dir.join("operator_pubkey.pem").exists());
}
