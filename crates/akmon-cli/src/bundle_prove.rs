//! `akmon bundle prove-openssl` — emit the artifacts a third party needs to verify an Akmon
//! Ed25519 signature with stock `openssl` alone (no Akmon binary, no cloud).
//!
//! This is the reproducible proof of Akmon's offline-verifiability wedge (metric F.1). It is
//! READ-ONLY on the bundle: it reconstructs the canonical `AGEF-SIG-v1` statement via the locked
//! [`akmon_bundle::signing_statement`], extracts the already-present detached signature from
//! `manifest.signatures[]`, and re-encodes the supplied public key as SPKI PEM. It signs nothing
//! and writes nothing back into the bundle — it only writes three side files into `--out-dir`.

use std::path::PathBuf;
use std::process::ExitCode;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, SCHEME_ED25519, SIG_STATEMENT_VERSION,
    ed25519_spki_pem, key_id, parse_public_key_hex, read_bundle, signing_statement,
};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::bundle_cmd::{
    BundleImportFormat, bundle_export_output_display, bundle_read_bundle_error_category,
    bundle_read_bundle_exit_code,
};

/// File names of the three artifacts written into `--out-dir`.
const STATEMENT_FILE: &str = "statement.bin";
const SIGNATURE_FILE: &str = "signature.bin";
const PUBKEY_PEM_FILE: &str = "pubkey.pem";

/// Length in bytes of a raw Ed25519 detached signature.
const ED25519_SIGNATURE_LEN: usize = 64;

/// Arguments for `akmon bundle prove-openssl`.
#[derive(Args, Debug, Clone)]
pub struct BundleProveArgs {
    /// Path to the signed `.akmon` bundle file.
    pub(crate) bundle: PathBuf,
    /// File containing the signer's raw Ed25519 public key as 64 hex characters — the same
    /// artifact `akmon bundle sign` prints and `akmon bundle verify --verify-key` consumes.
    #[arg(long = "verify-key", value_name = "HEX_FILE")]
    pub(crate) verify_key: PathBuf,
    /// Directory to write the three verification artifacts into. Defaults to the current directory.
    #[arg(long = "out-dir", value_name = "DIR", default_value = ".")]
    pub(crate) out_dir: PathBuf,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleImportFormat,
}

/// Stable JSON shape for `akmon bundle prove-openssl --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleProveReportV1 {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this report.
    akmon_version: String,
    /// Path to the bundle the artifacts were extracted from.
    bundle_path: String,
    /// Session UUID from the bundle manifest.
    session_id: String,
    /// Signer key id (lowercase hex SHA-256 of the public key) the signature was matched on.
    key_id: String,
    /// Path of the emitted statement bytes (the exact message that was signed).
    statement_path: String,
    /// Path of the emitted 64-byte raw detached signature.
    signature_path: String,
    /// Path of the emitted SPKI PEM public key.
    pubkey_pem_path: String,
    /// The exact `openssl` command a third party runs to verify, with real emitted paths.
    openssl_command: String,
}

/// Emits `statement.bin`, `signature.bin`, and `pubkey.pem` for offline `openssl` verification.
pub fn run_bundle_prove_openssl(args: &BundleProveArgs) -> ExitCode {
    let json = matches!(args.format, BundleImportFormat::Json);
    let fail = |category: &str, msg: String, code: u8| -> ExitCode {
        if json {
            let _ = print_prove_json_error(category, msg);
        } else {
            eprintln!("akmon: bundle prove-openssl: {msg}");
        }
        ExitCode::from(code)
    };

    // 1. Read the bundle (read-only; same path run_bundle_verify uses).
    let mut file = match std::fs::File::open(&args.bundle) {
        Ok(f) => f,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot open bundle {}: {err}", args.bundle.display()),
                3,
            );
        }
    };
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = match read_bundle(&mut file, &options) {
        Ok(c) => c,
        Err(err) => {
            let category = bundle_read_bundle_error_category(&err);
            let code = bundle_read_bundle_exit_code(&err);
            return fail(category, err.to_string(), code);
        }
    };
    drop(file);
    let manifest = &contents.manifest;

    // 2. Parse the signer public key from --verify-key (same contract as bundle verify).
    let pubkey = match std::fs::read_to_string(&args.verify_key) {
        Ok(hex_str) => match parse_public_key_hex(&hex_str) {
            Ok(pk) => pk,
            Err(err) => {
                return fail(
                    "verify_key_error",
                    format!("--verify-key {}: {err}", args.verify_key.display()),
                    3,
                );
            }
        },
        Err(err) => {
            return fail(
                "verify_key_error",
                format!(
                    "cannot read --verify-key {}: {err}",
                    args.verify_key.display()
                ),
                3,
            );
        }
    };
    let expected_key_id = key_id(&pubkey);

    // 3. Select the manifest signature whose key_id matches the supplied public key.
    let Some(signatures) = &manifest.signatures else {
        return fail(
            "no_matching_signature",
            format!(
                "bundle has no signatures; cannot prove a signature for key_id {expected_key_id}"
            ),
            1,
        );
    };
    let Some(entry) = signatures.iter().find(|s| s.key_id == expected_key_id) else {
        return fail(
            "no_matching_signature",
            format!(
                "no manifest signature matches the supplied public key (expected key_id {expected_key_id})"
            ),
            1,
        );
    };

    // 4. The selected entry must be an AGEF-SIG-v1 Ed25519 signature.
    if entry.scheme != SCHEME_ED25519 || entry.statement_version != SIG_STATEMENT_VERSION {
        return fail(
            "unsupported_signature",
            format!(
                "signature for key_id {expected_key_id} is scheme={} statement_version={}; only ed25519/AGEF-SIG-v1 is supported",
                entry.scheme, entry.statement_version
            ),
            1,
        );
    }

    // 5. Decode the 64-byte raw detached signature.
    let signature_bytes = match hex::decode(&entry.signature) {
        Ok(b) => b,
        Err(err) => {
            return fail(
                "malformed_signature",
                format!("signature for key_id {expected_key_id} is not valid hex: {err}"),
                1,
            );
        }
    };
    if signature_bytes.len() != ED25519_SIGNATURE_LEN {
        return fail(
            "malformed_signature",
            format!(
                "signature for key_id {expected_key_id} is {} bytes; expected {ED25519_SIGNATURE_LEN}",
                signature_bytes.len()
            ),
            1,
        );
    }

    // 6. Reconstruct the exact signed statement via the LOCKED library function.
    let statement = signing_statement(
        &manifest.agef_version,
        &manifest.hash_algorithm,
        &manifest.session.id,
        &manifest.session.head,
    );

    // 7. Encode the supplied public key as SPKI PEM (the form stock openssl ingests).
    let pubkey_pem = match ed25519_spki_pem(&pubkey) {
        Ok(pem) => pem,
        Err(err) => {
            return fail(
                "verify_key_error",
                format!("cannot encode public key as PEM: {err}"),
                3,
            );
        }
    };

    // 8. Write the three artifacts into --out-dir (raw bytes; no newline translation).
    if let Err(err) = std::fs::create_dir_all(&args.out_dir) {
        return fail(
            "io_error",
            format!("cannot create --out-dir {}: {err}", args.out_dir.display()),
            3,
        );
    }
    let statement_path = args.out_dir.join(STATEMENT_FILE);
    let signature_path = args.out_dir.join(SIGNATURE_FILE);
    let pubkey_pem_path = args.out_dir.join(PUBKEY_PEM_FILE);
    if let Err(err) = std::fs::write(&statement_path, statement.as_bytes()) {
        return fail(
            "io_error",
            format!("cannot write {}: {err}", statement_path.display()),
            3,
        );
    }
    if let Err(err) = std::fs::write(&signature_path, &signature_bytes) {
        return fail(
            "io_error",
            format!("cannot write {}: {err}", signature_path.display()),
            3,
        );
    }
    if let Err(err) = std::fs::write(&pubkey_pem_path, pubkey_pem.as_bytes()) {
        return fail(
            "io_error",
            format!("cannot write {}: {err}", pubkey_pem_path.display()),
            3,
        );
    }

    let pubkey_pem_disp = bundle_export_output_display(pubkey_pem_path.as_path());
    let statement_disp = bundle_export_output_display(statement_path.as_path());
    let signature_disp = bundle_export_output_display(signature_path.as_path());
    let openssl_command = format!(
        "openssl pkeyutl -verify -pubin -inkey {pubkey_pem_disp} -rawin -in {statement_disp} -sigfile {signature_disp}"
    );

    match args.format {
        BundleImportFormat::Json => {
            let report = BundleProveReportV1 {
                tool: "akmon".to_owned(),
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                bundle_path: bundle_export_output_display(args.bundle.as_path()),
                session_id: manifest.session.id.clone(),
                key_id: expected_key_id,
                statement_path: statement_disp,
                signature_path: signature_disp,
                pubkey_pem_path: pubkey_pem_disp,
                openssl_command,
            };
            match serde_json::to_string_pretty(&report) {
                Ok(s) => println!("{s}"),
                Err(err) => {
                    return fail(
                        "io_error",
                        format!("failed to render JSON output: {err}"),
                        3,
                    );
                }
            }
        }
        BundleImportFormat::Human => {
            eprintln!(
                "wrote openssl verification artifacts for: {}",
                bundle_export_output_display(args.bundle.as_path())
            );
            eprintln!("  session_id: {}", manifest.session.id);
            eprintln!("  key_id: {expected_key_id}");
            eprintln!("  statement: {statement_disp}");
            eprintln!("  signature: {signature_disp}");
            eprintln!("  public key (PEM): {pubkey_pem_disp}");
            eprintln!("  verify offline with OpenSSL 3.x (no Akmon binary):");
            eprintln!("    {openssl_command}");
        }
    }
    ExitCode::SUCCESS
}

/// JSON shape emitted when `prove-openssl` cannot complete.
#[derive(Debug, Serialize, Deserialize)]
struct BundleProveError {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
}

fn print_prove_json_error(category: &str, error: String) -> std::io::Result<()> {
    let body = BundleProveError {
        tool: "akmon".to_owned(),
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        error,
        category: category.to_owned(),
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}
