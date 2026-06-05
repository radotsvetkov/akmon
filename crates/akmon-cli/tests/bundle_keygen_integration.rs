//! Integration tests for `akmon bundle keygen` (D-18 signing-key generation, AGEF v0.1.2).
//!
//! These prove that keygen writes a genuinely usable PKCS#8 v2 DER key (the keygen -> sign -> verify
//! roundtrip), enforces 0600 perms on unix, refuses to clobber an existing private key without
//! --force, and emits exactly the 64-hex public key to --public-out.

use std::process::Command;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, key_id, parse_public_key_hex,
    public_key_from_pkcs8, read_bundle, verify_manifest_signatures,
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

/// keygen writes the key file and the surfaced public_key_hex / key_id match the bytes on disk.
#[test]
fn t_keygen_writes_key_and_surfaces_pubkey() {
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("signer.pk8");

    let run = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &out.display().to_string(),
            "--format",
            "json",
        ])
        .output()
        .expect("run bundle keygen");
    assert_eq!(
        run.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(out.is_file(), "key file should exist");

    let report: Value = serde_json::from_slice(&run.stdout).expect("keygen json");
    let reported_hex = report
        .get("public_key_hex")
        .and_then(Value::as_str)
        .expect("public_key_hex");
    let reported_key_id = report
        .get("key_id")
        .and_then(Value::as_str)
        .expect("key_id");
    assert_eq!(reported_hex.len(), 64, "public key hex is 64 chars");
    assert_eq!(report.get("tool").and_then(Value::as_str), Some("akmon"));
    // public_out is present-as-null when --public-out was not given.
    assert_eq!(report.get("public_out"), Some(&Value::Null));

    let file_bytes = std::fs::read(&out).expect("read key bytes");
    let pubkey = public_key_from_pkcs8(&file_bytes).expect("derive pubkey from on-disk key");
    assert_eq!(reported_hex, hex::encode(&pubkey));
    assert_eq!(reported_key_id, key_id(&pubkey));
}

/// The private key file must be 0600 on unix, set at create time. This is the load-bearing
/// security assertion for a signing tool. Covers both the fresh-create and --force-over-0644 cases.
#[cfg(unix)]
#[test]
fn t_keygen_unix_perms_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("signer.pk8");

    let run = akmon()
        .args(["bundle", "keygen", "--out", &out.display().to_string()])
        .output()
        .expect("run bundle keygen");
    assert_eq!(run.status.code(), Some(0));
    let mode = std::fs::metadata(&out)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "fresh-create key must be 0600");

    // Now create a wide-perms file and --force over it; perms must still come out 0600.
    let wide = dir.path().join("wide.pk8");
    std::fs::write(&wide, b"placeholder").expect("write placeholder");
    std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o644)).expect("chmod 644");
    let forced = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &wide.display().to_string(),
            "--force",
        ])
        .output()
        .expect("run bundle keygen --force");
    assert_eq!(forced.status.code(), Some(0));
    let mode = std::fs::metadata(&wide)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "--force over 0644 must re-assert 0600");
}

/// The proof the generated key is genuinely usable PKCS#8 v2: keygen -> sign -> verify end to end.
#[test]
fn t_keygen_roundtrip_sign_verify() {
    let dir = tempdir().expect("tempdir");
    let journal_dir = dir.path();
    let session_id = Uuid::new_v4();
    create_clean_session(journal_dir, session_id);

    // Export an unsigned bundle.
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

    // Generate the key with keygen (this is the thing under test producing the key).
    let key_path = journal_dir.join("signer.pk8");
    let pub_path = journal_dir.join("signer.pub.hex");
    let keygen = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &key_path.display().to_string(),
            "--public-out",
            &pub_path.display().to_string(),
        ])
        .output()
        .expect("run bundle keygen");
    assert_eq!(
        keygen.status.code(),
        Some(0),
        "keygen stderr={}",
        String::from_utf8_lossy(&keygen.stderr)
    );

    // Sign the bundle with the generated key.
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
    assert_eq!(
        sign.status.code(),
        Some(0),
        "sign stderr={}",
        String::from_utf8_lossy(&sign.stderr)
    );
    let sign_report: Value = serde_json::from_slice(&sign.stdout).expect("sign json");
    assert_eq!(
        sign_report.get("signature_count").and_then(Value::as_u64),
        Some(1)
    );

    // Verify with the keygen-produced public key.
    let verify = akmon()
        .args([
            "bundle",
            "verify",
            &bundle.display().to_string(),
            "--verify-key",
            &pub_path.display().to_string(),
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
    let v: Value = serde_json::from_slice(&verify.stdout).expect("verify json");
    let sigs = v
        .get("signatures")
        .and_then(Value::as_array)
        .expect("signatures");
    assert_eq!(
        sigs[0].get("outcome").and_then(Value::as_str),
        Some("verified")
    );

    // In-process proof too: re-read the bundle and verify the manifest signatures directly.
    let pub_hex = std::fs::read_to_string(&pub_path).expect("read pub hex");
    let pubkey = parse_public_key_hex(&pub_hex).expect("parse pubkey hex");
    let mut f = std::fs::File::open(&bundle).expect("open signed bundle");
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = read_bundle(&mut f, &options).expect("read signed bundle");
    let report = verify_manifest_signatures(&contents.manifest, &[pubkey]);
    assert!(report.any_verified(), "manifest signature must verify");
    assert!(!report.any_invalid());
}

/// A second keygen to the same --out without --force is refused (exit 3) and the original key is
/// byte-for-byte unchanged.
#[test]
fn t_keygen_no_clobber_refuses_without_force() {
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("signer.pk8");

    let first = akmon()
        .args(["bundle", "keygen", "--out", &out.display().to_string()])
        .output()
        .expect("first keygen");
    assert_eq!(first.status.code(), Some(0));
    let original = std::fs::read(&out).expect("read original key");

    let second = akmon()
        .args(["bundle", "keygen", "--out", &out.display().to_string()])
        .output()
        .expect("second keygen");
    assert_eq!(
        second.status.code(),
        Some(3),
        "no-clobber must exit 3; stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    let after = std::fs::read(&out).expect("read key after refusal");
    assert_eq!(original, after, "original private key must be untouched");
}

/// `--force` overwrites the existing key with a fresh (different) one; on unix it stays 0600.
#[test]
fn t_keygen_force_overwrites() {
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("signer.pk8");

    let first = akmon()
        .args(["bundle", "keygen", "--out", &out.display().to_string()])
        .output()
        .expect("first keygen");
    assert_eq!(first.status.code(), Some(0));
    let original = std::fs::read(&out).expect("read original key");

    let forced = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &out.display().to_string(),
            "--force",
        ])
        .output()
        .expect("forced keygen");
    assert_eq!(forced.status.code(), Some(0));
    let after = std::fs::read(&out).expect("read key after force");
    assert_ne!(original, after, "force must produce a fresh, different key");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "forced key must still be 0600");
    }
}

/// `--public-out` writes exactly the 64-hex public key with no trailing newline, matching the bytes
/// derived from the private key, and parses back via parse_public_key_hex.
#[test]
fn t_keygen_public_out_is_64_hex() {
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("k.pk8");
    let pub_out = dir.path().join("k.pub.hex");

    let run = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &out.display().to_string(),
            "--public-out",
            &pub_out.display().to_string(),
        ])
        .output()
        .expect("run keygen");
    assert_eq!(run.status.code(), Some(0));

    let contents = std::fs::read_to_string(&pub_out).expect("read public-out");
    assert_eq!(contents.len(), 64, "public-out is exactly 64 chars");
    assert!(
        contents.chars().all(|c| c.is_ascii_hexdigit()),
        "public-out is all hex"
    );
    assert!(
        !contents.ends_with('\n'),
        "public-out must not have a trailing newline"
    );

    let key_bytes = std::fs::read(&out).expect("read key");
    let pubkey = public_key_from_pkcs8(&key_bytes).expect("derive pubkey");
    assert_eq!(contents, hex::encode(&pubkey));
    assert_eq!(
        parse_public_key_hex(&contents).expect("parse public-out"),
        pubkey
    );
}

/// A pre-existing --public-out blocks the run BEFORE the private key is written, so a refused run
/// leaves no half-written --out behind.
#[test]
fn t_keygen_public_out_no_clobber() {
    let dir = tempdir().expect("tempdir");
    let out = dir.path().join("k.pk8");
    let pub_out = dir.path().join("k.pub.hex");
    std::fs::write(&pub_out, "preexisting").expect("seed public-out");

    let run = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &out.display().to_string(),
            "--public-out",
            &pub_out.display().to_string(),
        ])
        .output()
        .expect("run keygen");
    assert_eq!(
        run.status.code(),
        Some(3),
        "public-out clobber must exit 3; stderr={}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        !out.exists(),
        "private key must NOT be written when --public-out clobber is refused"
    );
    assert_eq!(
        std::fs::read_to_string(&pub_out).expect("read public-out"),
        "preexisting",
        "pre-existing public-out must be untouched"
    );

    // With --force the same run succeeds.
    let forced = akmon()
        .args([
            "bundle",
            "keygen",
            "--out",
            &out.display().to_string(),
            "--public-out",
            &pub_out.display().to_string(),
            "--force",
        ])
        .output()
        .expect("run keygen --force");
    assert_eq!(forced.status.code(), Some(0));
    assert!(out.is_file(), "private key written under --force");
}
