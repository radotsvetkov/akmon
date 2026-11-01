//! Rejection-path tests for hostile or malformed bundle archives through `akmon bundle verify`
//! (TC-06, TC-07, TC-08, TC-10, TC-13, TC-16, TC-17).
//!
//! Archives are assembled by hand (a minimal ustar writer inside a raw-block zstd frame) so the
//! tests can produce byte layouts `write_bundle` never would — duplicate entries, symlinks,
//! uppercase object names, out-of-range timestamps — without adding test dependencies.

use std::collections::{BTreeMap, HashMap};
use std::process::Command;

use akmon_bundle::manifest::{Manifest, Producer, SessionMetadata};
use akmon_bundle::{EventsWriter, WriteBundleOptions, write_bundle};
use akmon_journal::{AGEF_SPEC_VERSION, Event, EventKind, Hash, HashAlgorithm, digest_bytes};
use serde_json::Value;
use tempfile::tempdir;

#[allow(dead_code)]
mod common;
use common::akmon_bin_path;

fn akmon() -> Command {
    Command::new(akmon_bin_path())
}

const START_TS: i64 = 1_700_000_000;

fn object(byte: u8) -> (Hash, Vec<u8>) {
    let bytes = vec![byte; 8];
    (digest_bytes(HashAlgorithm::Sha256, &bytes), bytes)
}

/// A valid two-event session: (manifest, events, objects).
fn valid_parts() -> (Manifest, Vec<Event>, HashMap<Hash, Vec<u8>>) {
    let (cwd_hash, cwd_bytes) = object(0x11);
    let (config_hash, config_bytes) = object(0x12);
    let start = Event {
        parents: vec![],
        kind: EventKind::SessionStart {
            cwd_hash: cwd_hash.clone(),
            config_hash: config_hash.clone(),
        },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(START_TS).expect("ts"),
        sequence: 0,
    };
    let start_hash = start.content_hash(HashAlgorithm::Sha256).expect("hash");
    let end = Event {
        parents: vec![start_hash],
        kind: EventKind::SessionEnd { summary_hash: None },
        emitted_at: time::OffsetDateTime::from_unix_timestamp(START_TS + 1).expect("ts"),
        sequence: 1,
    };
    let end_hash = end.content_hash(HashAlgorithm::Sha256).expect("hash");
    let manifest = Manifest {
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
    };
    let objects = HashMap::from([(cwd_hash, cwd_bytes), (config_hash, config_bytes)]);
    (manifest, vec![start, end], objects)
}

fn events_bin(events: &[Event]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut writer = EventsWriter::new(&mut out);
    for e in events {
        writer.write_event(e).expect("write event");
    }
    writer.finish().expect("finish");
    out
}

/// One ustar header block for `name` (regular file `b'0'`, symlink `b'2'`, directory `b'5'`).
fn ustar_header(name: &str, size: usize, typeflag: u8, linkname: &str) -> [u8; 512] {
    let mut h = [0_u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..107].copy_from_slice(b"0000644");
    h[108..115].copy_from_slice(b"0000000");
    h[116..123].copy_from_slice(b"0000000");
    h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    h[136..147].copy_from_slice(b"00000000000");
    h[156] = typeflag;
    h[157..157 + linkname.len()].copy_from_slice(linkname.as_bytes());
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// A tar archive of (name, typeflag, linkname, bytes) entries.
fn tar(entries: &[(&str, u8, &str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, typeflag, link, bytes) in entries {
        out.extend_from_slice(&ustar_header(name, bytes.len(), *typeflag, link));
        out.extend_from_slice(bytes);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0_u8; 1024]);
    out
}

/// Wraps `data` in a single zstd frame made only of raw (uncompressed) blocks.
fn zstd_raw(data: &[u8]) -> Vec<u8> {
    const MAX_BLOCK: usize = 128 * 1024;
    let mut out = vec![0x28, 0xB5, 0x2F, 0xFD, 0xA0];
    out.extend_from_slice(&u32::try_from(data.len()).expect("small").to_le_bytes());
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(MAX_BLOCK).collect()
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let last = u32::from(i + 1 == chunks.len());
        let header = (u32::try_from(chunk.len()).expect("small") << 3) | last;
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(chunk);
    }
    out
}

/// Standard entries for a valid bundle, before any hostile additions.
fn standard_entries(
    manifest: &Manifest,
    events: &[u8],
    objects: &HashMap<Hash, Vec<u8>>,
) -> Vec<(String, u8, String, Vec<u8>)> {
    let mut entries = vec![
        (
            "manifest.json".to_owned(),
            b'0',
            String::new(),
            manifest.to_canonical_json_bytes().expect("manifest"),
        ),
        (
            "events.bin".to_owned(),
            b'0',
            String::new(),
            events.to_vec(),
        ),
    ];
    let mut sorted: Vec<_> = objects.iter().collect();
    sorted.sort_by_key(|(h, _)| h.to_hex());
    for (hash, bytes) in sorted {
        entries.push((
            format!("objects/{}", hash.to_hex()),
            b'0',
            String::new(),
            bytes.clone(),
        ));
    }
    entries
}

fn archive(entries: &[(String, u8, String, Vec<u8>)]) -> Vec<u8> {
    let borrowed: Vec<(&str, u8, &str, &[u8])> = entries
        .iter()
        .map(|(n, t, l, b)| (n.as_str(), *t, l.as_str(), b.as_slice()))
        .collect();
    zstd_raw(&tar(&borrowed))
}

/// Runs `akmon bundle verify --format json` on raw archive bytes.
fn verify_json(bytes: &[u8], extra: &[&str]) -> (Option<i32>, Value) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("b.akmon");
    std::fs::write(&path, bytes).expect("write");
    let out = akmon()
        .args(["bundle", "verify"])
        .arg(&path)
        .args(["--format", "json"])
        .args(extra)
        .output()
        .expect("run akmon");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "json: {e}; stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code(), v)
}

fn assert_tool_error(v: &Value, category: &str) {
    assert_eq!(v["category"], category, "body={v}");
    assert_eq!(v["passed"], false, "body={v}");
    assert_eq!(v["tool"], "akmon", "body={v}");
    assert!(v["akmon_version"].is_string(), "body={v}");
    assert!(v["error"].is_string(), "body={v}");
}

#[test]
fn t_hand_built_valid_archive_passes() {
    let (manifest, events, objects) = valid_parts();
    let bytes = archive(&standard_entries(&manifest, &events_bin(&events), &objects));
    let (code, v) = verify_json(&bytes, &[]);
    assert_eq!(code, Some(0), "report={v}");
    assert_eq!(v["integrity_ok"], true);
    assert_eq!(
        v["warnings"].as_array().map(Vec::len),
        Some(0),
        "report={v}"
    );
}

#[test]
fn t_duplicate_manifest_entry_rejected() {
    let (manifest, events, objects) = valid_parts();
    let mut entries = standard_entries(&manifest, &events_bin(&events), &objects);
    entries.push(entries[0].clone());
    let (code, v) = verify_json(&archive(&entries), &[]);
    assert_eq!(code, Some(1));
    assert_tool_error(&v, "invalid_archive");
}

#[test]
fn t_duplicate_events_bin_entry_rejected() {
    let (manifest, events, objects) = valid_parts();
    let mut entries = standard_entries(&manifest, &events_bin(&events), &objects);
    entries.push(entries[1].clone());
    let (code, v) = verify_json(&archive(&entries), &[]);
    assert_eq!(code, Some(1));
    assert_tool_error(&v, "invalid_archive");
}

#[test]
fn t_duplicate_object_entry_rejected() {
    let (manifest, events, objects) = valid_parts();
    let mut entries = standard_entries(&manifest, &events_bin(&events), &objects);
    let dup = entries[2].clone();
    entries.push(dup);
    let (code, v) = verify_json(&archive(&entries), &[]);
    assert_eq!(code, Some(1));
    assert_tool_error(&v, "invalid_archive");
}

#[test]
fn t_uppercase_object_name_rejected() {
    let (manifest, events, objects) = valid_parts();
    let mut entries = standard_entries(&manifest, &events_bin(&events), &objects);
    entries[2].0 = entries[2].0.to_uppercase().replace("OBJECTS/", "objects/");
    let (code, v) = verify_json(&archive(&entries), &[]);
    assert_eq!(code, Some(1));
    assert_tool_error(&v, "invalid_archive");
}

#[test]
fn t_symlink_entry_rejected_in_strict_mode() {
    let (manifest, events, objects) = valid_parts();
    let mut entries = standard_entries(&manifest, &events_bin(&events), &objects);
    entries.push((
        "evil".to_owned(),
        b'2',
        "/etc/passwd".to_owned(),
        Vec::new(),
    ));
    let (code, v) = verify_json(&archive(&entries), &[]);
    assert_eq!(code, Some(1));
    assert_tool_error(&v, "unknown_bundle_file");

    // --allow-extra-files keeps the documented lenient behaviour (entry ignored).
    let (code, v) = verify_json(&archive(&entries), &["--allow-extra-files"]);
    assert_eq!(code, Some(0), "report={v}");
}

#[test]
fn t_unreferenced_object_fails_verification() {
    let (mut manifest, events, mut objects) = valid_parts();
    let (extra_hash, extra_bytes) = object(0x77);
    objects.insert(extra_hash.clone(), extra_bytes);
    manifest.object_count = 3;
    let bytes = archive(&standard_entries(&manifest, &events_bin(&events), &objects));
    let (code, v) = verify_json(&bytes, &[]);
    assert_eq!(code, Some(1), "report={v}");
    assert_eq!(v["passed"], false);
    assert_eq!(v["integrity_ok"], false);
    let violations = v["violations"].as_array().expect("violations");
    assert!(
        violations
            .iter()
            .any(|x| x["category"] == "unreferenced_object"
                && x["object_hash"] == extra_hash.to_hex().as_str()),
        "report={v}"
    );
}

#[test]
fn t_out_of_range_event_timestamp_rejected() {
    let (manifest, events, objects) = valid_parts();
    let mut ev = events_bin(&events);
    // Rewrite the first event's emitted_at (CBOR tag 1, u32 START_TS) to i64::MAX — still
    // canonical CBOR, so only the out-of-range check can catch it.
    let needle: Vec<u8> = [&[0xC1_u8, 0x1A][..], &(START_TS as u32).to_be_bytes()].concat();
    let pos = ev
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
        .expect("timestamp encoding present");
    let replacement: Vec<u8> = [&[0xC1_u8, 0x1B][..], &i64::MAX.to_be_bytes()].concat();
    ev.splice(pos..pos + needle.len(), replacement);
    let frame_len = u32::from_be_bytes([ev[0], ev[1], ev[2], ev[3]]) + 4;
    ev[..4].copy_from_slice(&frame_len.to_be_bytes());
    let bytes = archive(&standard_entries(&manifest, &ev, &objects));
    let (code, v) = verify_json(&bytes, &[]);
    assert_eq!(code, Some(1), "body={v}");
    assert_tool_error(&v, "malformed_framing");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("timestamp out of range")),
        "body={v}"
    );
}

#[test]
fn t_truncated_archive_exits_1_not_3() {
    let (manifest, events, objects) = valid_parts();
    let mut out = Vec::new();
    write_bundle(
        &mut out,
        &manifest,
        &events,
        &objects,
        &WriteBundleOptions::default(),
    )
    .expect("write");
    out.truncate(out.len() / 2);
    let (code, v) = verify_json(&out, &[]);
    assert_eq!(
        code,
        Some(1),
        "a corrupt archive is a failed verification: {v}"
    );
    assert_eq!(v["passed"], false);
    assert!(
        matches!(
            v["category"].as_str(),
            Some("invalid_archive" | "invalid_compression")
        ),
        "body={v}"
    );
}

#[test]
fn t_missing_bundle_file_exits_3_with_shared_error_shape() {
    let dir = tempdir().expect("tempdir");
    let out = akmon()
        .args(["bundle", "verify"])
        .arg(dir.path().join("nope.akmon"))
        .args(["--format", "json"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(3));
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_tool_error(&v, "io_error");
}

#[test]
fn t_max_decoded_bytes_flag_caps_decompression() {
    let (manifest, events, objects) = valid_parts();
    let bytes = archive(&standard_entries(&manifest, &events_bin(&events), &objects));
    let (code, v) = verify_json(&bytes, &["--max-decoded-bytes", "600"]);
    assert_eq!(code, Some(1));
    assert_tool_error(&v, "bundle_too_large");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("--max-decoded-bytes")),
        "body={v}"
    );
    let (code, v) = verify_json(&bytes, &["--max-decoded-bytes", "10000000"]);
    assert_eq!(code, Some(0), "report={v}");
}
