//! Reproducible-proof integration test for `akmon bundle prove-openssl` (metric F.1).
//!
//! The whole point of the command is that a third party holding ONLY `openssl` (no Akmon binary)
//! can cryptographically verify an Akmon Ed25519 signature. This test signs a real bundle, emits
//! the verification artifacts, and — only when a genuinely Ed25519-capable `openssl` is present —
//! runs the real openssl verify (positive) and a tampered-statement case (negative).
//!
//! It SKIPS gracefully (eprintln a note + `return`, never asserts/panics) when no capable openssl
//! is available. This is REQUIRED: the default macOS `/usr/bin/openssl` is LibreSSL, which lacks
//! `-rawin` and cannot load Ed25519 keys, so a presence-only gate would turn CI RED. Point at a
//! capable binary with `AKMON_OPENSSL=<path>`; otherwise the test probes `openssl` on PATH.

use std::path::Path;
use std::process::Command;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, ed25519_spki_pem, generate_pkcs8,
    parse_public_key_hex, read_bundle, signing_statement,
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

/// Exports an unsigned bundle from a clean session, signs it, and returns
/// (bundle_path, pubkey_hex_file_path).
fn build_signed_bundle(journal_dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
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
    assert!(
        export.status.success(),
        "export stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );

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
    assert!(
        sign.status.success(),
        "sign stderr={}",
        String::from_utf8_lossy(&sign.stderr)
    );
    let report: Value = serde_json::from_slice(&sign.stdout).expect("sign json");
    let pubkey_hex = report
        .get("public_key_hex")
        .and_then(Value::as_str)
        .expect("public_key_hex")
        .to_owned();
    let pubkey_file = journal_dir.join("signer.pub.hex");
    std::fs::write(&pubkey_file, &pubkey_hex).expect("write pubkey hex");
    (bundle, pubkey_file)
}

/// Deterministic, no-openssl: the emitted artifacts are structurally correct and statement.bin is
/// byte-identical to the library `signing_statement(...)` output. This locks the proof even on the
/// LibreSSL CI default where the openssl legs below skip.
#[test]
fn t_prove_openssl_emits_byte_identical_artifacts() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let (bundle, pubkey_file) = build_signed_bundle(journal_dir);

    let out_dir = journal_dir.join("proof");
    let prove = akmon()
        .args([
            "bundle",
            "prove-openssl",
            &bundle.display().to_string(),
            "--verify-key",
            &pubkey_file.display().to_string(),
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
    assert_eq!(report.get("tool").and_then(Value::as_str), Some("akmon"));

    let statement_path = out_dir.join("statement.bin");
    let signature_path = out_dir.join("signature.bin");
    let pubkey_pem_path = out_dir.join("pubkey.pem");

    // (1) statement.bin == library signing_statement(...) over the manifest (byte identity).
    let mut f = std::fs::File::open(&bundle).expect("open signed bundle");
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = read_bundle(&mut f, &options).expect("read signed bundle");
    let m = &contents.manifest;
    let expected_statement = signing_statement(
        &m.agef_version,
        &m.hash_algorithm,
        &m.session.id,
        &m.session.head,
    );
    let statement_bytes = std::fs::read(&statement_path).expect("read statement.bin");
    assert_eq!(
        statement_bytes,
        expected_statement.as_bytes(),
        "statement.bin must be byte-identical to signing_statement(...)"
    );

    // (4) signature.bin is exactly 64 bytes; pubkey.pem is the 44-byte-DER SPKI PEM.
    let signature_bytes = std::fs::read(&signature_path).expect("read signature.bin");
    assert_eq!(
        signature_bytes.len(),
        64,
        "raw Ed25519 signature is 64 bytes"
    );
    let pubkey_hex = std::fs::read_to_string(&pubkey_file).expect("read pubkey hex");
    let pubkey = parse_public_key_hex(&pubkey_hex).expect("parse pubkey");
    let pem_bytes = std::fs::read_to_string(&pubkey_pem_path).expect("read pubkey.pem");
    assert_eq!(
        pem_bytes,
        ed25519_spki_pem(&pubkey).expect("pem"),
        "pubkey.pem must equal the library SPKI PEM"
    );
    assert!(pem_bytes.starts_with("-----BEGIN PUBLIC KEY-----\n"));
    assert!(pem_bytes.ends_with("-----END PUBLIC KEY-----\n"));
}

/// The reproducible proof: a third party with ONLY openssl verifies the signature (positive), and
/// a one-byte tamper of the statement makes openssl fail (negative). Skips when openssl cannot do
/// Ed25519 (LibreSSL CI default) — never fails CI in that case.
#[test]
fn t_prove_openssl_verifies_and_tamper_fails() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();

    let probe_dir = journal_dir.join("probe");
    std::fs::create_dir_all(&probe_dir).expect("mkdir probe");
    if !openssl_can_verify_ed25519(&probe_dir) {
        eprintln!(
            "SKIP t_prove_openssl_verifies_and_tamper_fails: no Ed25519-capable openssl found \
             (probed `{}`). Set AKMON_OPENSSL=<OpenSSL 3.x path> to run the real proof. \
             The deterministic artifact test still covered byte-identity.",
            openssl_bin()
        );
        return;
    }

    let (bundle, pubkey_file) = build_signed_bundle(journal_dir);
    let out_dir = journal_dir.join("proof");
    let prove = akmon()
        .args([
            "bundle",
            "prove-openssl",
            &bundle.display().to_string(),
            "--verify-key",
            &pubkey_file.display().to_string(),
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
    let openssl_command = report
        .get("openssl_command")
        .and_then(Value::as_str)
        .expect("openssl_command");
    // The printed command names the standard verify invocation (used verbatim below).
    assert!(openssl_command.contains("pkeyutl -verify -pubin -inkey"));
    assert!(openssl_command.contains("-rawin"));

    let bin = openssl_bin();
    let statement_path = out_dir.join("statement.bin");
    let signature_path = out_dir.join("signature.bin");
    let pubkey_pem_path = out_dir.join("pubkey.pem");

    // POSITIVE: openssl verifies the emitted artifacts (exit 0).
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
        "openssl should verify the emitted artifacts; stdout={} stderr={}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );

    // NEGATIVE: flip one byte of statement.bin -> openssl verification must fail (non-zero).
    let mut tampered = std::fs::read(&statement_path).expect("read statement");
    let last = tampered.last_mut().expect("non-empty statement");
    *last ^= 0x01;
    let tampered_path = out_dir.join("statement.tampered.bin");
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
        "openssl must reject a tampered statement; stdout={} stderr={}",
        String::from_utf8_lossy(&verify_bad.stdout),
        String::from_utf8_lossy(&verify_bad.stderr)
    );
}

/// Error contract: supplying a public key that signed nothing in this bundle -> exit 1,
/// category `no_matching_signature`.
#[test]
fn t_prove_openssl_no_matching_signature() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let (bundle, _signer_pub) = build_signed_bundle(journal_dir);

    // A different, unrelated public key.
    let other_pub =
        akmon_bundle::public_key_from_pkcs8(&generate_pkcs8().expect("keygen")).expect("pubkey");
    let other_pub_file = journal_dir.join("other.pub.hex");
    std::fs::write(&other_pub_file, hex::encode(&other_pub)).expect("write other pub");

    let out_dir = journal_dir.join("proof");
    let prove = akmon()
        .args([
            "bundle",
            "prove-openssl",
            &bundle.display().to_string(),
            "--verify-key",
            &other_pub_file.display().to_string(),
            "--out-dir",
            &out_dir.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle prove-openssl");
    assert_eq!(prove.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&prove.stdout).expect("error json");
    assert_eq!(
        report.get("category").and_then(Value::as_str),
        Some("no_matching_signature")
    );
}
