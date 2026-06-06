//! `akmon bundle attest` — record a signed operator attestation on an AGEF bundle (decision D-20,
//! AGEF v0.1.3 §A.15).
//!
//! This is the CLI surface for operator identity: it answers "who claims to have operated this
//! session" independently of the bundle's integrity hash chain and of any head signature. It reads
//! the bundle, builds a signed `AGEF-OPERATOR-v1` attestation over the manifest's session head plus
//! the four operator identity fields with the supplied PKCS#8 Ed25519 key, appends it to
//! `manifest.operator_attestations[]`, and writes the bundle back atomically (in place or to
//! `--output`).
//!
//! It is purely additive: the `AGEF-SIG-v1` head statement, any existing head signatures, and the
//! `prove-openssl` bytes stay byte-untouched. In particular it NEVER rewrites `agef_version` when the
//! bundle already carries head signatures, because that would invalidate the existing
//! `AGEF-SIG-v1` signature (O9). The private key is consumed but never printed.

use std::path::PathBuf;
use std::process::ExitCode;

use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, OperatorIdentity, ReadBundleOptions, SigningError,
    WriteBundleOptions, build_operator_attestation, key_id, public_key_from_pkcs8, read_bundle,
    write_bundle,
};
use akmon_journal::AGEF_SPEC_VERSION;
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::bundle_cmd::{
    BundleImportFormat, bundle_export_output_display, bundle_read_bundle_error_category,
    bundle_read_bundle_exit_code,
};

/// Arguments for `akmon bundle attest`.
#[derive(Args, Debug, Clone)]
pub struct BundleAttestArgs {
    /// Path to the `.akmon` bundle file to attest.
    pub(crate) bundle: PathBuf,
    /// Path to a PKCS#8 v2 Ed25519 private key (raw DER), as produced by
    /// `akmon bundle keygen --out`. (`openssl genpkey` emits PKCS#8 v1, which is rejected.)
    #[arg(long)]
    pub(crate) key: PathBuf,
    /// Stable operator identifier (signed): an email, employee id, or service account. Required.
    #[arg(long = "operator-id", value_name = "ID")]
    pub(crate) operator_id: String,
    /// Human-readable display name of the operator (signed). Defaults to empty.
    #[arg(long = "display-name", default_value = "", value_name = "NAME")]
    pub(crate) display_name: String,
    /// Role the operator acted in for this session (signed), for example `approver`. Defaults to
    /// empty.
    #[arg(long, default_value = "", value_name = "ROLE")]
    pub(crate) role: String,
    /// Organization the operator belongs to (signed). Defaults to empty.
    #[arg(long, default_value = "", value_name = "ORG")]
    pub(crate) org: String,
    /// Destination for the attested bundle. Defaults to attesting the input bundle in place.
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleImportFormat,
}

/// Stable JSON shape for `akmon bundle attest --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleAttestReportV1 {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this report.
    akmon_version: String,
    /// Path the attested bundle was written to.
    bundle_path: String,
    /// Session UUID from the bundle manifest.
    session_id: String,
    /// The self-asserted operator identifier that was signed.
    operator_id: String,
    /// The self-asserted role that was signed.
    role: String,
    /// Attester key id: lowercase hex SHA-256 of the operator public key.
    key_id: String,
    /// The operator public key as 64 lowercase hex characters (safe to distribute to verifiers).
    public_key_hex: String,
    /// Path the attested bundle was written to (canonicalized for display).
    output_path: String,
}

/// Reads a bundle, builds a signed operator attestation, appends it, and writes the bundle back.
pub fn run_bundle_attest(args: &BundleAttestArgs) -> ExitCode {
    let json = matches!(args.format, BundleImportFormat::Json);
    let fail = |category: &str, msg: String, code: u8| -> ExitCode {
        if json {
            let _ = print_attest_json_error(category, msg);
        } else {
            eprintln!("akmon: bundle attest: {msg}");
        }
        ExitCode::from(code)
    };

    // 1. Read the bundle strictly (same path run_bundle_sign uses).
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
    let mut contents = match read_bundle(&mut file, &options) {
        Ok(c) => c,
        Err(err) => {
            let category = bundle_read_bundle_error_category(&err);
            let code = bundle_read_bundle_exit_code(&err);
            return fail(category, err.to_string(), code);
        }
    };
    drop(file);

    // 2. Read the raw PKCS#8 key bytes (same error categories run_bundle_sign uses).
    let pkcs8 = match std::fs::read(&args.key) {
        Ok(bytes) => bytes,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot read --key {}: {err}", args.key.display()),
                3,
            );
        }
    };
    let public_key = match public_key_from_pkcs8(&pkcs8) {
        Ok(pk) => pk,
        Err(err) => {
            return fail(
                "invalid_key",
                format!("--key {}: {err}", args.key.display()),
                2,
            );
        }
    };

    // 3. AGEF_VERSION rule (O9): stamp the current spec version ONLY when there is no head
    //    signature to invalidate. If head signatures already exist, leave agef_version untouched so
    //    the already-present AGEF-SIG-v1 signature stays valid. The attestation is built from the
    //    manifest's current agef_version either way, so it is self-consistent.
    if contents
        .manifest
        .signatures
        .as_ref()
        .is_none_or(Vec::is_empty)
    {
        contents.manifest.agef_version = AGEF_SPEC_VERSION.to_owned();
    }

    // 4. Build the signed operator attestation over the manifest's current head + identity.
    let created_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let identity = OperatorIdentity {
        operator_id: args.operator_id.clone(),
        display_name: args.display_name.clone(),
        role: args.role.clone(),
        org: args.org.clone(),
    };
    let attestation =
        match build_operator_attestation(&contents.manifest, &identity, &pkcs8, &created_at) {
            Ok(att) => att,
            Err(SigningError::IllegalOperatorField) => {
                return fail(
                    "invalid_operator_field",
                    "operator identity field is empty (operator-id) or contains a newline/carriage \
                     return; such values are rejected to prevent statement injection"
                        .to_owned(),
                    2,
                );
            }
            Err(err) => {
                return fail(
                    "invalid_key",
                    format!("--key {}: {err}", args.key.display()),
                    2,
                );
            }
        };

    let kid = key_id(&public_key);
    let public_key_hex = hex::encode(&public_key);

    // 5. Append the attestation to manifest.operator_attestations[].
    contents
        .manifest
        .operator_attestations
        .get_or_insert_with(Vec::new)
        .push(attestation);

    // 6. Write atomically (temp + rename) so a failed write never clobbers the input bundle.
    let out_path = args.output.clone().unwrap_or_else(|| args.bundle.clone());
    let tmp_path = out_path.with_extension("akmon-attest-tmp");
    let mut out_file = match std::fs::File::create(&tmp_path) {
        Ok(f) => f,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot create temp file {}: {err}", tmp_path.display()),
                3,
            );
        }
    };
    if let Err(err) = write_bundle(
        &mut out_file,
        &contents.manifest,
        &contents.events,
        &contents.objects,
        &WriteBundleOptions::default(),
    ) {
        drop(out_file);
        let _ = std::fs::remove_file(&tmp_path);
        return fail("bundle_error", format!("bundle write failed: {err}"), 3);
    }
    drop(out_file);
    if let Err(err) = std::fs::rename(&tmp_path, &out_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return fail(
            "io_error",
            format!(
                "cannot finalize attested bundle {}: {err}",
                out_path.display()
            ),
            3,
        );
    }

    // 7. Surface the result (files now exist, so paths can be canonicalized).
    let output_display = bundle_export_output_display(out_path.as_path());
    match args.format {
        BundleImportFormat::Json => {
            let report = BundleAttestReportV1 {
                tool: "akmon".to_owned(),
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                bundle_path: out_path.display().to_string(),
                session_id: contents.manifest.session.id.clone(),
                operator_id: args.operator_id.clone(),
                role: args.role.clone(),
                key_id: kid,
                public_key_hex,
                output_path: output_display,
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
            eprintln!("attested bundle: {output_display}");
            eprintln!("  session_id: {}", contents.manifest.session.id);
            eprintln!("  operator_id: {}", args.operator_id);
            eprintln!("  role: {}", args.role);
            eprintln!("  key_id: {kid}");
            eprintln!("  operator public key (hex): {public_key_hex}");
            eprintln!(
                "  KEEP THE PRIVATE KEY SECRET. Distribute only the public key (hex) to verifiers."
            );
            eprintln!("  verify this attestation by distributing the public key and running:");
            eprintln!(
                "    agef-verify {output_display} --operator-key <pubkey-hex-file> --require-operator"
            );
        }
    }
    ExitCode::SUCCESS
}

/// JSON shape emitted when `attest` cannot complete (identical contract to the sibling commands).
#[derive(Debug, Serialize, Deserialize)]
struct BundleAttestError {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
}

fn print_attest_json_error(category: &str, error: String) -> std::io::Result<()> {
    let body = BundleAttestError {
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
