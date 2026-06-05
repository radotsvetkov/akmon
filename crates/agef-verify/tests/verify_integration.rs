//! Integration tests for the `agef-verify` binary.

use std::collections::{BTreeMap, HashMap};
use std::process::Command;

use akmon_bundle::manifest::{Manifest, ManifestSignature, Producer, SessionMetadata};
use akmon_bundle::{
    BundleContents, SCHEME_ED25519, SIG_STATEMENT_VERSION, WriteBundleOptions, generate_pkcs8,
    key_id, public_key_from_pkcs8, sign_statement, signing_statement, write_bundle,
};
use akmon_journal::{AGEF_SPEC_VERSION, Event, EventKind, Hash, HashAlgorithm, digest_bytes};
use tempfile::tempdir;

fn algo() -> HashAlgorithm {
    HashAlgorithm::Sha256
}

fn object(byte: u8) -> (Hash, Vec<u8>) {
    let bytes = vec![byte; 8];
    (digest_bytes(algo(), &bytes), bytes)
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
    let start_hash = start.content_hash(algo()).expect("start hash");

    let end = Event {
        parents: vec![start_hash.clone()],
        kind: EventKind::SessionEnd { summary_hash: None },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_001).expect("ts"),
        sequence: 1,
    };
    let end_hash = end.content_hash(algo()).expect("end hash");

    let objects = HashMap::from([(cwd_hash, cwd_bytes), (config_hash, config_bytes)]);
    let manifest = Manifest {
        agef_version: AGEF_SPEC_VERSION.to_owned(),
        producer: Producer {
            name: "akmon".to_owned(),
            version: "test".to_owned(),
        },
        session: SessionMetadata {
            id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            head: end_hash.to_hex(),
            created_at: "2026-05-04T14:00:00Z".to_owned(),
            ended_at: "2026-05-04T14:01:00Z".to_owned(),
        },
        hash_algorithm: "sha256".to_owned(),
        object_count: 2,
        event_count: 2,
        signatures: None,
        extra: BTreeMap::new(),
    };

    BundleContents {
        manifest,
        events: vec![start, end],
        objects,
    }
}

fn write_bundle_file(path: &std::path::Path, contents: &BundleContents) {
    let mut out = Vec::new();
    write_bundle(
        &mut out,
        &contents.manifest,
        &contents.events,
        &contents.objects,
        &WriteBundleOptions::default(),
    )
    .expect("write bundle");
    std::fs::write(path, out).expect("write file");
}

fn agef_verify_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agef-verify"))
}

#[test]
fn t_verify_clean_bundle_human_exits_0() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("session.akmon");
    write_bundle_file(&bundle, &valid_bundle());

    let out = agef_verify_bin()
        .arg(&bundle)
        .output()
        .expect("run agef-verify");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn t_verify_clean_bundle_json_passed() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("session.akmon");
    write_bundle_file(&bundle, &valid_bundle());

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run agef-verify");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(true));
    assert_eq!(
        v.get("violations").and_then(|x| x.as_array()).map(Vec::len),
        Some(0)
    );
}

#[test]
fn t_verify_tampered_bundle_exits_1() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("session.akmon");
    let mut contents = valid_bundle();
    contents.manifest.event_count = 99;
    write_bundle_file(&bundle, &contents);

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run agef-verify");
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(false));
}

/// Signs `contents`' head with a fresh Ed25519 key, populating `manifest.signatures[]`.
/// Returns the signer's public key as hex (for `--verify-key`).
fn sign_bundle_head(contents: &mut BundleContents) -> String {
    let pkcs8 = generate_pkcs8().expect("keygen");
    let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
    let m = &contents.manifest;
    let statement = signing_statement(
        &m.agef_version,
        &m.hash_algorithm,
        &m.session.id,
        &m.session.head,
    );
    let sig = sign_statement(statement.as_bytes(), &pkcs8).expect("sign");
    contents.manifest.signatures = Some(vec![ManifestSignature {
        scheme: SCHEME_ED25519.to_owned(),
        key_id: key_id(&pubkey),
        statement_version: SIG_STATEMENT_VERSION.to_owned(),
        signature: hex::encode(&sig),
        created_at: "2026-05-04T14:01:00Z".to_owned(),
    }]);
    hex::encode(&pubkey)
}

#[test]
fn t_verify_signed_bundle_with_key_verifies() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("signed.akmon");
    let key_file = dir.path().join("signer.pub.hex");
    let mut contents = valid_bundle();
    let pubkey_hex = sign_bundle_head(&mut contents);
    write_bundle_file(&bundle, &contents);
    std::fs::write(&key_file, pubkey_hex).expect("write key");

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json"])
        .arg("--verify-key")
        .arg(&key_file)
        .output()
        .expect("run agef-verify");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(true));
    let sigs = v
        .get("signatures")
        .and_then(|x| x.as_array())
        .expect("signatures");
    assert_eq!(sigs.len(), 1);
    assert_eq!(
        sigs[0].get("outcome").and_then(|o| o.as_str()),
        Some("verified")
    );
}

#[test]
fn t_verify_signed_bundle_without_key_is_unverified() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("signed.akmon");
    let mut contents = valid_bundle();
    let _ = sign_bundle_head(&mut contents);
    write_bundle_file(&bundle, &contents);

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run agef-verify");
    // No --verify-key: integrity passes and exit is 0; the signature is reported as unverified.
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let sigs = v
        .get("signatures")
        .and_then(|x| x.as_array())
        .expect("signatures");
    assert_eq!(
        sigs[0].get("outcome").and_then(|o| o.as_str()),
        Some("unverified_no_key")
    );
}

#[test]
fn t_require_signature_without_key_fails() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("signed.akmon");
    let mut contents = valid_bundle();
    let _ = sign_bundle_head(&mut contents);
    write_bundle_file(&bundle, &contents);

    let out = agef_verify_bin()
        .arg(&bundle)
        .arg("--require-signature")
        .output()
        .expect("run agef-verify");
    assert_eq!(out.status.code(), Some(1));
}

/// Builds a clean bundle whose `SessionStart` config object is an OTEL config object with the
/// given `capture_level` (mirrors `akmon_otel::objects::config_object_bytes` byte shape; the
/// importer is not needed — only the right object bytes matter to the verifier).
fn otel_bundle(capture_level: &str) -> BundleContents {
    let (cwd_hash, cwd_bytes) = object(0x11);
    let config_bytes = serde_json::to_vec(&serde_json::json!({
        "akmon_otel_config": true,
        "schema": "akmon-otel-config-v1",
        "capture_level": capture_level,
        "source_semconv": "1.37.0",
        "provider": "openai",
        "model": "gpt-4o",
        "conversation_id": serde_json::Value::Null,
        "agent": serde_json::Value::Null,
    }))
    .expect("config json");
    let config_hash = digest_bytes(algo(), &config_bytes);

    let start = Event {
        parents: vec![],
        kind: EventKind::SessionStart {
            cwd_hash: cwd_hash.clone(),
            config_hash: config_hash.clone(),
        },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("ts"),
        sequence: 0,
    };
    let start_hash = start.content_hash(algo()).expect("start hash");
    let end = Event {
        parents: vec![start_hash],
        kind: EventKind::SessionEnd { summary_hash: None },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_001).expect("ts"),
        sequence: 1,
    };
    let end_hash = end.content_hash(algo()).expect("end hash");

    let objects = HashMap::from([(cwd_hash, cwd_bytes), (config_hash, config_bytes)]);
    let manifest = Manifest {
        agef_version: AGEF_SPEC_VERSION.to_owned(),
        producer: Producer {
            name: "akmon".to_owned(),
            version: "test".to_owned(),
        },
        session: SessionMetadata {
            id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            head: end_hash.to_hex(),
            created_at: "2026-05-04T14:00:00Z".to_owned(),
            ended_at: "2026-05-04T14:01:00Z".to_owned(),
        },
        hash_algorithm: "sha256".to_owned(),
        object_count: 2,
        event_count: 2,
        signatures: None,
        extra: BTreeMap::new(),
    };
    BundleContents {
        manifest,
        events: vec![start, end],
        objects,
    }
}

/// (a) A structural OTEL bundle surfaces `capture.level == "structural"`; integrity still passes.
#[test]
fn t_structural_otel_bundle_surfaces_capture() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("structural.akmon");
    write_bundle_file(&bundle, &otel_bundle("structural"));

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json"])
        .output()
        .expect("run agef-verify");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(true));
    assert_eq!(
        v.pointer("/capture/level").and_then(|x| x.as_str()),
        Some("structural")
    );
    assert_eq!(
        v.pointer("/capture/source_semconv")
            .and_then(|x| x.as_str()),
        Some("1.37.0")
    );
}

/// (a) `--require-capture full` rejects a structural OTEL bundle (exit 1, passed == false).
#[test]
fn t_structural_otel_bundle_fails_require_capture_full() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("structural.akmon");
    write_bundle_file(&bundle, &otel_bundle("structural"));

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json", "--require-capture", "full"])
        .output()
        .expect("run agef-verify");
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(false));
    let categories: Vec<&str> = v
        .get("violations")
        .and_then(|x| x.as_array())
        .expect("violations")
        .iter()
        .filter_map(|x| x.get("category").and_then(|c| c.as_str()))
        .collect();
    assert!(
        categories.contains(&"capture_requirement_unmet"),
        "categories={categories:?}"
    );
}

/// (b) A full OTEL bundle passes `--require-capture full` (exit 0).
#[test]
fn t_full_otel_bundle_passes_require_capture_full() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("full.akmon");
    write_bundle_file(&bundle, &otel_bundle("full"));

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json", "--require-capture", "full"])
        .output()
        .expect("run agef-verify");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(true));
    assert_eq!(
        v.pointer("/capture/level").and_then(|x| x.as_str()),
        Some("full")
    );
}

/// (c) A native bundle (no OTEL config) has `capture == null` and passes `--require-capture full`.
#[test]
fn t_native_bundle_capture_null_and_passes_require_capture() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("native.akmon");
    write_bundle_file(&bundle, &valid_bundle());

    let out = agef_verify_bin()
        .arg(&bundle)
        .args(["--format", "json", "--require-capture", "full"])
        .output()
        .expect("run agef-verify");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v.get("passed").and_then(|p| p.as_bool()), Some(true));
    // `capture` is omitted (skip_serializing_if None) for native bundles → absent or null.
    assert!(
        v.get("capture").map(|c| c.is_null()).unwrap_or(true),
        "capture should be null/absent for native bundle, got {:?}",
        v.get("capture")
    );
}
