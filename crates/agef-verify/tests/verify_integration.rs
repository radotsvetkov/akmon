//! Integration tests for the `agef-verify` binary.

use std::collections::{BTreeMap, HashMap};
use std::process::Command;

use akmon_bundle::manifest::{Manifest, Producer, SessionMetadata};
use akmon_bundle::{BundleContents, WriteBundleOptions, write_bundle};
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
