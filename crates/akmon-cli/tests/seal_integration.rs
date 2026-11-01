//! Integration tests for `akmon seal`: OTEL trace / journal session -> signed, self-verified bundle
//! plus `<OUTPUT>.pub`, driven through the real `akmon` binary on the checked-in fixture.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use akmon_bundle::generate_pkcs8;
use serde_json::Value;
use tempfile::tempdir;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::*;

fn akmon() -> Command {
    Command::new(akmon_bin_path())
}

/// Absolute path to the checked-in real-framework legacy fixture (content-off => structural).
fn fixture_path() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/openai_v2_weather_legacy.otlp.json"
    )
    .to_owned()
}

fn s(path: &Path) -> String {
    path.display().to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Writes a fresh PKCS#8 v2 Ed25519 key (the form `bundle keygen --out` emits) under `dir`.
fn write_key(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, generate_pkcs8().expect("keygen")).expect("write key");
    path
}

/// Runs `akmon seal <input> --key <key> --journal <dir>/journal` plus `extra` args.
fn seal(dir: &Path, input: &str, key: &Path, extra: &[&str]) -> Output {
    akmon()
        .arg("seal")
        .arg(input)
        .arg("--key")
        .arg(key)
        .arg("--journal")
        .arg(dir.join("journal"))
        .args(extra)
        .output()
        .expect("run akmon seal")
}

/// Runs `akmon bundle verify <bundle> --format json` plus `extra` args.
fn verify_json(bundle: &Path, extra: &[&str]) -> (Option<i32>, Value) {
    let out = akmon()
        .args(["bundle", "verify", &s(bundle), "--format", "json"])
        .args(extra)
        .output()
        .expect("run bundle verify");
    let json = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    (out.status.code(), json)
}

/// Every non-directory entry under `dir` (recursive), excluding the journal and key files.
fn produced_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name == "journal" || name.ends_with(".pk8") {
            continue;
        }
        if path.is_dir() {
            found.extend(produced_files(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[test]
fn seal_trace_produces_bundle_and_pub_that_verify_with_require_signature() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = write_key(tmp, "signer.pk8");
    let out = tmp.join("audit.akmon");

    let sealed = seal(tmp, &fixture_path(), &key, &["--output", &s(&out)]);
    assert_eq!(sealed.status.code(), Some(0), "stderr={}", stderr(&sealed));
    let err = stderr(&sealed);
    assert!(err.contains("capture level: structural"), "stderr={err}");
    assert!(
        err.contains(&format!(
            "agef-verify {} --verify-key {}.pub --require-signature",
            s(&out),
            s(&out)
        )),
        "stderr={err}"
    );
    assert!(
        err.contains("--format markdown > report.md"),
        "stderr={err}"
    );
    assert!(err.contains("separate trusted channel"), "stderr={err}");

    let pub_path = tmp.join("audit.akmon.pub");
    assert!(pub_path.is_file());
    assert!(!tmp.join("audit.akmon.operator.pub").exists());

    let (code, json) = verify_json(
        &out,
        &["--verify-key", &s(&pub_path), "--require-signature"],
    );
    assert_eq!(code, Some(0), "verify json={json}");
    assert_eq!(json["passed"], Value::Bool(true));
    assert_eq!(json["signature_verified"], Value::Bool(true));
    assert_eq!(
        json.pointer("/capture/level"),
        Some(&Value::from("structural"))
    );

    // No temp files are left next to the output.
    let leftovers: Vec<_> = produced_files(tmp)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("seal-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
}

#[test]
fn seal_pub_file_is_byte_identical_to_keygen_public_out() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = tmp.join("signer.pk8");
    let keygen_pub = tmp.join("signer.pub");
    let keygen = akmon()
        .args(["bundle", "keygen", "--out", &s(&key), "--public-out"])
        .arg(&keygen_pub)
        .output()
        .expect("run keygen");
    assert!(keygen.status.success(), "stderr={}", stderr(&keygen));

    let out = tmp.join("audit.akmon");
    let sealed = seal(tmp, &fixture_path(), &key, &["--output", &s(&out)]);
    assert_eq!(sealed.status.code(), Some(0), "stderr={}", stderr(&sealed));
    assert_eq!(
        std::fs::read(&keygen_pub).expect("keygen pub"),
        std::fs::read(tmp.join("audit.akmon.pub")).expect("seal pub")
    );
}

#[test]
fn seal_with_operator_attestation_verifies_with_require_operator_key() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = write_key(tmp, "signer.pk8");
    let op_key = write_key(tmp, "op.pk8");
    let out = tmp.join("audit.akmon");

    let sealed = seal(
        tmp,
        &fixture_path(),
        &key,
        &[
            "--output",
            &s(&out),
            "--operator-key",
            &s(&op_key),
            "--operator-id",
            "alice@example.com",
            "--display-name",
            "Alice",
            "--role",
            "approver",
            "--org",
            "Example GmbH",
        ],
    );
    assert_eq!(sealed.status.code(), Some(0), "stderr={}", stderr(&sealed));
    let op_pub = tmp.join("audit.akmon.operator.pub");
    assert!(op_pub.is_file());

    let (code, json) = verify_json(
        &out,
        &[
            "--verify-key",
            &s(&tmp.join("audit.akmon.pub")),
            "--require-signature",
            "--require-operator-key",
            &s(&op_pub),
        ],
    );
    assert_eq!(code, Some(0), "verify json={json}");
    assert_eq!(json["operator_key_verified"], Value::Bool(true));
    assert_eq!(
        json.pointer("/operators/0/operator_id"),
        Some(&Value::from("alice@example.com"))
    );
    assert_eq!(
        json.pointer("/operators/0/role"),
        Some(&Value::from("approver"))
    );
}

#[test]
fn seal_existing_session_id() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let sid = Uuid::new_v4();
    create_clean_session(&tmp.join("journal"), sid);
    let key = write_key(tmp, "signer.pk8");
    let out = tmp.join("native.akmon");

    let sealed = seal(
        tmp,
        &sid.to_string(),
        &key,
        &["--output", &s(&out), "--format", "json"],
    );
    assert_eq!(sealed.status.code(), Some(0), "stderr={}", stderr(&sealed));
    let json: Value = serde_json::from_slice(&sealed.stdout).expect("seal json");
    assert_eq!(json["session_id"], Value::from(sid.to_string()));
    assert_eq!(json["capture_level"], Value::from("full"));

    let (code, report) = verify_json(
        &out,
        &[
            "--verify-key",
            &s(&tmp.join("native.akmon.pub")),
            "--require-signature",
        ],
    );
    assert_eq!(code, Some(0), "verify json={report}");
    assert_eq!(report["session_id"], Value::from(sid.to_string()));
}

#[test]
fn seal_unknown_session_id_is_bad_input() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    create_clean_session(&tmp.join("journal"), Uuid::new_v4());
    let key = write_key(tmp, "signer.pk8");
    let sealed = seal(
        tmp,
        &Uuid::new_v4().to_string(),
        &key,
        &["--output", &s(&tmp.join("x.akmon"))],
    );
    assert_eq!(sealed.status.code(), Some(2), "stderr={}", stderr(&sealed));
    assert!(produced_files(tmp).is_empty());
}

#[test]
fn seal_refuses_overwrite_without_force_and_replaces_with_force() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = write_key(tmp, "signer.pk8");
    let out = tmp.join("audit.akmon");
    std::fs::write(&out, b"precious").expect("write existing");

    let refused = seal(
        tmp,
        &fixture_path(),
        &key,
        &["--output", &s(&out), "--format", "json"],
    );
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stderr={}",
        stderr(&refused)
    );
    let err: Value = serde_json::from_slice(&refused.stdout).expect("error json");
    assert_eq!(err["category"], Value::from("output_exists"));
    assert_eq!(err["passed"], Value::Bool(false));
    assert_eq!(std::fs::read(&out).expect("read"), b"precious");
    assert!(!tmp.join("audit.akmon.pub").exists());
    // Refused before doing any work: the trace was not even imported.
    assert!(!tmp.join("journal").exists());

    let forced = seal(
        tmp,
        &fixture_path(),
        &key,
        &["--output", &s(&out), "--force"],
    );
    assert_eq!(forced.status.code(), Some(0), "stderr={}", stderr(&forced));
    let (code, _) = verify_json(
        &out,
        &[
            "--verify-key",
            &s(&tmp.join("audit.akmon.pub")),
            "--require-signature",
        ],
    );
    assert_eq!(code, Some(0));
}

#[test]
fn seal_nonexistent_and_garbage_input_exit_2() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = write_key(tmp, "signer.pk8");

    let missing = seal(
        tmp,
        &s(&tmp.join("missing.json")),
        &key,
        &["--output", &s(&tmp.join("a.akmon"))],
    );
    assert_eq!(
        missing.status.code(),
        Some(2),
        "stderr={}",
        stderr(&missing)
    );

    let garbage = tmp.join("garbage.json");
    std::fs::write(&garbage, b"this is not json {").expect("write garbage");
    let bad = seal(
        tmp,
        &s(&garbage),
        &key,
        &["--output", &s(&tmp.join("b.akmon")), "--format", "json"],
    );
    assert_eq!(bad.status.code(), Some(2), "stderr={}", stderr(&bad));
    let err: Value = serde_json::from_slice(&bad.stdout).expect("error json");
    assert_eq!(err["tool"], Value::from("akmon seal"));
    assert!(err["category"].is_string());

    let files: Vec<_> = produced_files(tmp)
        .into_iter()
        .filter(|p| p != &garbage)
        .collect();
    assert!(files.is_empty(), "unexpected files: {files:?}");
}

#[test]
fn seal_json_success_shape() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = write_key(tmp, "signer.pk8");
    let op_key = write_key(tmp, "op.pk8");
    let out = tmp.join("audit.akmon");
    let sealed = seal(
        tmp,
        &fixture_path(),
        &key,
        &[
            "--output",
            &s(&out),
            "--operator-key",
            &s(&op_key),
            "--operator-id",
            "ops@example.com",
            "--format",
            "json",
        ],
    );
    assert_eq!(sealed.status.code(), Some(0), "stderr={}", stderr(&sealed));
    let json: Value = serde_json::from_slice(&sealed.stdout).expect("seal json");
    assert_eq!(json["tool"], Value::from("akmon seal"));
    assert_eq!(
        json["akmon_version"],
        Value::from(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(json["passed"], Value::Bool(true));
    assert!(Uuid::parse_str(json["session_id"].as_str().expect("sid")).is_ok());
    assert!(
        json["bundle"]
            .as_str()
            .expect("bundle")
            .ends_with("audit.akmon")
    );
    let signer_hex = json["signer_public_key"].as_str().expect("signer key");
    assert_eq!(signer_hex.len(), 64);
    assert_eq!(
        std::fs::read_to_string(tmp.join("audit.akmon.pub")).expect("pub"),
        signer_hex
    );
    assert_eq!(json["signer_key_id"].as_str().expect("kid").len(), 64);
    assert_eq!(
        json["operator_public_key"].as_str().expect("op key"),
        std::fs::read_to_string(tmp.join("audit.akmon.operator.pub")).expect("op pub")
    );
    assert_eq!(json["capture_level"], Value::from("structural"));
    assert!(json.get("proof_dir").is_none());
    let cmd = json["verify_command"].as_str().expect("verify_command");
    assert!(cmd.starts_with("agef-verify "), "{cmd}");
    assert!(cmd.contains("--require-signature"), "{cmd}");
    assert!(cmd.contains("--require-operator-key"), "{cmd}");
}

#[test]
fn seal_proof_dir_matches_bundle_prove_openssl() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    let key = write_key(tmp, "signer.pk8");
    let out = tmp.join("audit.akmon");
    let seal_proof = tmp.join("seal-proof");

    let sealed = seal(
        tmp,
        &fixture_path(),
        &key,
        &[
            "--output",
            &s(&out),
            "--proof-dir",
            &s(&seal_proof),
            "--format",
            "json",
        ],
    );
    assert_eq!(sealed.status.code(), Some(0), "stderr={}", stderr(&sealed));
    let json: Value = serde_json::from_slice(&sealed.stdout).expect("seal json");
    assert!(
        json["proof_dir"]
            .as_str()
            .expect("proof_dir")
            .ends_with("seal-proof")
    );

    let prove_dir = tmp.join("prove-proof");
    let prove = akmon()
        .args(["bundle", "prove-openssl", &s(&out), "--verify-key"])
        .arg(tmp.join("audit.akmon.pub"))
        .arg("--out-dir")
        .arg(&prove_dir)
        .output()
        .expect("run prove-openssl");
    assert!(prove.status.success(), "stderr={}", stderr(&prove));
    for name in ["statement.bin", "signature.bin", "pubkey.pem"] {
        assert_eq!(
            std::fs::read(seal_proof.join(name)).expect("seal artifact"),
            std::fs::read(prove_dir.join(name)).expect("prove artifact"),
            "{name} differs"
        );
    }
}

#[test]
fn seal_failure_leaves_no_files_behind() {
    let dir = tempdir().expect("tempdir");
    let tmp = dir.path();
    // An invalid signer key (not PKCS#8 v2): refused before any output is written.
    let bad_key = tmp.join("bad.pk8");
    std::fs::write(&bad_key, b"not a key").expect("write bad key");
    let out = tmp.join("audit.akmon");
    let proof = tmp.join("proof");
    let refused = seal(
        tmp,
        &fixture_path(),
        &bad_key,
        &["--output", &s(&out), "--proof-dir", &s(&proof)],
    );
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stderr={}",
        stderr(&refused)
    );
    assert!(produced_files(tmp).is_empty(), "{:?}", produced_files(tmp));

    // A journal session that cannot be exported (no SessionEnd) fails mid-pipeline: still no
    // bundle, no .pub, no proof artifacts, no temp files.
    let sid = Uuid::new_v4();
    create_session_missing_end(&tmp.join("journal"), sid);
    let key = write_key(tmp, "signer.pk8");
    let failed = seal(
        tmp,
        &sid.to_string(),
        &key,
        &["--output", &s(&out), "--proof-dir", &s(&proof)],
    );
    assert_ne!(failed.status.code(), Some(0), "stderr={}", stderr(&failed));
    assert!(produced_files(tmp).is_empty(), "{:?}", produced_files(tmp));
}
