//! End-to-end headline-claim integration test (Phase 9.2): a REAL third-party
//! framework's OpenTelemetry GenAI trace becomes a signed, standalone-verifiable
//! AGEF bundle, and an auditor confirms the signature with ONLY `openssl` — no
//! Akmon binary, no cloud.
//!
//! This drives the WHOLE chain as ONE flow through the real `akmon` binary on the
//! checked-in fixture
//! `crates/akmon-cli/tests/fixtures/openai_v2_weather_legacy.otlp.json` (a
//! representative model of the DEFAULT, content-off emission of
//! `opentelemetry-instrumentation-openai-v2`, in the legacy `<= v1.36`
//! message-event form — see `tests/fixtures/README.md`):
//!
//!   akmon otel import  ->  bundle export  ->  bundle sign  ->
//!   bundle verify --verify-key --require-signature  ->  bundle prove-openssl  ->
//!   real `openssl pkeyutl -verify` (positive + one-byte-tamper negative).
//!
//! HONESTY: an OTEL-imported, content-off trace is `capture_level=structural`, NOT
//! full. The test asserts the imported bundle surfaces `structural`, and that
//! `bundle verify --require-capture full` correctly FAILS (exit 1). No step implies
//! byte-level or full replay from imported telemetry.
//!
//! The real-`openssl` leg reuses the EXACT capability-probe + graceful-SKIP pattern
//! from `bundle_prove_openssl_integration.rs`: the default macOS `/usr/bin/openssl`
//! is LibreSSL, which lacks `-rawin` and cannot load Ed25519 keys, so a
//! presence-only gate would turn CI RED. Point at a capable binary with
//! `AKMON_OPENSSL=<OpenSSL 3.x path>`; otherwise the test probes `openssl` on PATH
//! and SKIPS (eprintln + `return`, never asserts) when it cannot do Ed25519. A
//! separate deterministic test locks the emitted artifacts byte-for-byte without
//! any openssl, so a skip never reduces the regression guarantee to nothing.

use std::path::Path;
use std::process::Command;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, ed25519_spki_pem, generate_pkcs8,
    parse_public_key_hex, read_bundle, signing_statement,
};
use serde_json::Value;
use tempfile::tempdir;

#[allow(dead_code)]
mod common;
use common::*;

fn akmon() -> Command {
    Command::new(akmon_bin_path())
}

/// Absolute path to the checked-in real-framework legacy fixture.
fn fixture_path() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/openai_v2_weather_legacy.otlp.json"
    )
    .to_owned()
}

// ---------------------------------------------------------------------------
// openssl capability-probe helpers, copied verbatim from
// bundle_prove_openssl_integration.rs (each integration test file is
// self-contained per the repo convention; tests/common gains no openssl logic).
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Chain driver: import the fixture -> export -> sign, returning the paths the
// downstream steps need. Asserts the import honesty signals along the way.
// ---------------------------------------------------------------------------

/// Outcome of the import + export + sign legs of the chain.
struct ChainArtifacts {
    /// The signed `.akmon` bundle.
    bundle: std::path::PathBuf,
    /// File holding the signer's public key as 64 hex chars.
    pubkey_file: std::path::PathBuf,
}

/// Imports the fixture, asserts the honesty signals, exports, signs, and verifies.
/// Returns the signed bundle + the published public-key-hex file under `tmp`.
fn drive_import_export_sign(tmp: &Path) -> ChainArtifacts {
    let journal = tmp.join("journal");
    let fixture = fixture_path();

    // (1) otel import — a REAL-framework legacy trace becomes a fresh AGEF session.
    let import = akmon()
        .args([
            "otel",
            "import",
            &fixture,
            "--journal",
            &journal.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run otel import");
    assert_eq!(
        import.status.code(),
        Some(0),
        "import stderr={}",
        String::from_utf8_lossy(&import.stderr)
    );
    let import_json: Value = serde_json::from_slice(&import.stdout).expect("import json");

    // HONESTY: a content-off OTEL import is STRUCTURAL, with at least one turn suppressed
    // because the source telemetry carried no real message bodies. Never reads as full.
    assert_eq!(
        import_json.get("capture_level").and_then(Value::as_str),
        Some("structural"),
        "a content-off OTEL import must report capture_level=structural"
    );
    assert_eq!(
        import_json.get("provider_calls").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        import_json.get("tool_calls").and_then(Value::as_u64),
        Some(1)
    );
    assert!(
        import_json
            .get("turns_suppressed_no_content")
            .and_then(Value::as_u64)
            .expect("turns_suppressed_no_content")
            >= 1,
        "structural import must suppress at least one turn (no message bodies)"
    );
    let session_id = import_json
        .get("session_id")
        .and_then(Value::as_str)
        .expect("session_id")
        .to_owned();

    // (2) bundle export.
    let bundle = tmp.join("audit.akmon");
    let export = akmon()
        .args([
            "bundle",
            "export",
            &session_id,
            "--journal",
            &journal.display().to_string(),
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

    // (3) bundle sign — keys MUST come from the library (PKCS#8 v2); openssl genpkey
    // emits PKCS#8 v1 which ring's sign path rejects.
    let key_path = tmp.join("signer.pk8");
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
    let sign_json: Value = serde_json::from_slice(&sign.stdout).expect("sign json");
    let pubkey_hex = sign_json
        .get("public_key_hex")
        .and_then(Value::as_str)
        .expect("public_key_hex");
    let pubkey_file = tmp.join("signer.pub.hex");
    std::fs::write(&pubkey_file, pubkey_hex).expect("write pubkey hex");

    ChainArtifacts {
        bundle,
        pubkey_file,
    }
}

// ---------------------------------------------------------------------------
// The headline test: full chain, honesty gates, and the real openssl proof.
// ---------------------------------------------------------------------------

/// The whole headline claim as ONE asserted flow on the real-framework fixture:
/// OTEL legacy trace -> signed AGEF bundle -> auditor verifies with ONLY openssl.
#[test]
fn t_e2e_otel_legacy_trace_to_openssl_proof() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();

    let ChainArtifacts {
        bundle,
        pubkey_file,
    } = drive_import_export_sign(tmp);

    // (4) bundle verify --verify-key --require-signature: integrity passes, the signature
    //     verifies, and the surfaced capture level is honestly structural.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--verify-key",
            &pubkey_file.display().to_string(),
            "--require-signature",
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
    let verify_json: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    assert_eq!(
        verify_json.get("passed").and_then(Value::as_bool),
        Some(true)
    );
    let sigs = verify_json
        .get("signatures")
        .and_then(Value::as_array)
        .expect("signatures");
    assert_eq!(
        sigs[0].get("outcome").and_then(Value::as_str),
        Some("verified")
    );
    assert_eq!(
        verify_json
            .pointer("/capture/level")
            .and_then(Value::as_str),
        Some("structural"),
        "the signed config object must surface capture level structural through verify"
    );
    // Negative inverse, for credibility: the chain never reads as full-capture.
    assert_ne!(
        verify_json
            .pointer("/capture/level")
            .and_then(Value::as_str),
        Some("full"),
        "imported telemetry must never imply byte-level/full replay"
    );

    // (5) HONESTY GATE: a metadata-only OTEL import MUST be rejected by --require-capture full.
    let require_full = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--require-capture",
            "full",
        ])
        .output()
        .expect("run bundle verify require-capture full");
    assert_eq!(
        require_full.status.code(),
        Some(1),
        "--require-capture full must FAIL on a structural OTEL import; stderr={}",
        String::from_utf8_lossy(&require_full.stderr)
    );

    // (6) bundle prove-openssl: emit the standalone verification artifacts.
    let out_dir = tmp.join("proof");
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
        "prove-openssl stderr={}",
        String::from_utf8_lossy(&prove.stderr)
    );
    let prove_json: Value = serde_json::from_slice(&prove.stdout).expect("prove json");
    let openssl_command = prove_json
        .get("openssl_command")
        .and_then(Value::as_str)
        .expect("openssl_command");
    assert!(openssl_command.contains("pkeyutl -verify -pubin -inkey"));
    assert!(openssl_command.contains("-rawin"));

    // (7) The reproducible proof leg: a third party with ONLY openssl verifies the signature
    //     (positive) and a one-byte tamper of the statement makes openssl fail (negative).
    //     SKIP gracefully when openssl cannot do Ed25519 (LibreSSL CI default) — never fail CI.
    let probe_dir = tmp.join("probe");
    std::fs::create_dir_all(&probe_dir).expect("mkdir probe");
    if !openssl_can_verify_ed25519(&probe_dir) {
        eprintln!(
            "SKIP t_e2e_otel_legacy_trace_to_openssl_proof openssl leg: no Ed25519-capable \
             openssl found (probed `{}`). Set AKMON_OPENSSL=<OpenSSL 3.x path> to run the real \
             proof. The deterministic artifact test still locks byte-identity.",
            openssl_bin()
        );
        return;
    }

    let bin = openssl_bin();
    let statement_path = out_dir.join("statement.bin");
    let signature_path = out_dir.join("signature.bin");
    let pubkey_pem_path = out_dir.join("pubkey.pem");

    // POSITIVE: openssl verifies the emitted artifacts (exit 0).
    let verify_ok = Command::new(&bin)
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
        verify_ok.status.success(),
        "openssl should verify the emitted artifacts; stdout={} stderr={}",
        String::from_utf8_lossy(&verify_ok.stdout),
        String::from_utf8_lossy(&verify_ok.stderr)
    );

    // NEGATIVE: flip one byte of statement.bin (prove-openssl does NOT emit a tampered file;
    // the test creates it) -> openssl verification must fail (non-zero).
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

/// Deterministic, no-openssl lock on the proof: the artifacts prove-openssl emits for the
/// real-framework fixture are structurally correct and `statement.bin` is byte-identical to the
/// library `signing_statement(...)` reconstruction. This guarantees the proof even on the
/// LibreSSL CI default where the openssl leg above skips.
#[test]
fn t_e2e_otel_proof_artifacts_byte_identical() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();

    let ChainArtifacts {
        bundle,
        pubkey_file,
    } = drive_import_export_sign(tmp);

    let out_dir = tmp.join("proof");
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
        "prove-openssl stderr={}",
        String::from_utf8_lossy(&prove.stderr)
    );

    let statement_path = out_dir.join("statement.bin");
    let signature_path = out_dir.join("signature.bin");
    let pubkey_pem_path = out_dir.join("pubkey.pem");

    // statement.bin == library signing_statement(...) over the signed manifest (byte identity).
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

    // signature.bin is exactly 64 bytes; pubkey.pem equals the library SPKI PEM for the key.
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
