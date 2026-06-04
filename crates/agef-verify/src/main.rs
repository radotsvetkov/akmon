//! Standalone AGEF bundle integrity verifier.
//!
//! Verifies portable `.akmon` bundles using [`akmon_bundle::verify_bundle`] without
//! the Akmon CLI, journal store, or agent runtime. Intended for auditors and CI
//! pipelines that need a minimal, separately distributable check.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_bundle::{
    BundleContents, BundleError, BundleVerificationReport, BundleViolation,
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, SignatureOutcome, SignatureVerificationReport,
    parse_public_key_hex, read_bundle, verify_bundle, verify_manifest_signatures,
};
use clap::{Parser, ValueEnum};
use serde::Serialize;

/// Command-line interface for `agef-verify`.
#[derive(Debug, Parser)]
#[command(
    name = "agef-verify",
    version,
    about = "Verify an AGEF .akmon bundle without the Akmon CLI"
)]
struct Cli {
    /// Path to a `.akmon` bundle file.
    bundle: PathBuf,
    /// Output format for verification results.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    /// Allow unknown files inside the bundle archive (non-fail-closed read).
    #[arg(long, default_value_t = false)]
    allow_extra_files: bool,
    /// Trusted Ed25519 public key as hex (64 chars), read from a file. Repeatable. When provided,
    /// each `manifest.signatures[]` entry is verified against these keys (AGEF v0.1.2).
    #[arg(long = "verify-key", value_name = "HEX_FILE")]
    verify_keys: Vec<PathBuf>,
    /// Fail (exit 1) unless at least one signature verifies against a `--verify-key`.
    #[arg(long, default_value_t = false)]
    require_signature: bool,
}

/// Human-readable or JSON output mode.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    /// Plain-text report on stderr/stdout.
    #[default]
    Human,
    /// `BundleVerifyReportV1`-compatible JSON on stdout.
    Json,
}

/// Machine-readable bundle verification result (compatible with `akmon bundle import --verify-only`).
#[derive(Debug, Serialize)]
struct BundleVerifyReportV1 {
    /// Verifier crate version (`agef-verify`); same field name as the Akmon CLI for automation.
    akmon_version: String,
    /// AGEF specification version declared by the bundle manifest.
    agef_version: String,
    /// Canonical path to the bundle file that was verified.
    bundle_path: String,
    /// Session UUID from the bundle manifest.
    session_id: String,
    /// Number of events decoded from `events.bin`.
    events_in_bundle: u64,
    /// Number of objects decoded from `objects/`.
    objects_in_bundle: u64,
    /// Holistic verdict: integrity clean, no invalid signatures, and (if `--require-signature`)
    /// at least one signature verified.
    passed: bool,
    /// Collected integrity violations (empty when structurally clean).
    violations: Vec<ReportViolation>,
    /// Per-signature verification results (empty when the bundle is unsigned).
    signatures: Vec<SignatureReport>,
}

/// One signature-verification result for JSON output.
#[derive(Debug, Serialize)]
struct SignatureReport {
    /// `key_id` from the manifest entry (hex SHA-256 of the signer public key).
    key_id: String,
    /// Signature scheme (`ed25519`).
    scheme: String,
    /// Outcome: `verified`, `invalid`, `unverified_no_key`, `unsupported_scheme`, or `malformed`.
    outcome: String,
}

/// Stable lowercase outcome string for [`SignatureReport::outcome`].
fn signature_outcome_str(outcome: &SignatureOutcome) -> &'static str {
    match outcome {
        SignatureOutcome::Verified => "verified",
        SignatureOutcome::Invalid => "invalid",
        SignatureOutcome::UnverifiedNoKey => "unverified_no_key",
        SignatureOutcome::UnsupportedScheme => "unsupported_scheme",
        SignatureOutcome::Malformed => "malformed",
    }
}

/// Maps the library signature report into CLI-compatible JSON entries.
fn signature_reports(report: &SignatureVerificationReport) -> Vec<SignatureReport> {
    report
        .checks
        .iter()
        .map(|c| SignatureReport {
            key_id: c.key_id.clone(),
            scheme: c.scheme.clone(),
            outcome: signature_outcome_str(&c.outcome).to_owned(),
        })
        .collect()
}

/// One bundle verification violation for JSON output.
#[derive(Debug, Serialize)]
struct ReportViolation {
    /// Stable category identifier.
    category: String,
    /// Event content hash in hex when applicable.
    event_hash: Option<String>,
    /// Object hash in hex when applicable.
    object_hash: Option<String>,
    /// Human-readable explanation.
    message: String,
}

/// JSON shape emitted when the bundle cannot be read or parsed.
#[derive(Debug, Serialize)]
struct VerifyInfraErrorV1 {
    /// Verifier tool name.
    tool: &'static str,
    /// Verifier crate version.
    tool_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
}

/// Resolves and validates a user-supplied bundle path before any read.
fn validated_bundle_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = dunce::canonicalize(path)
        .map_err(|err| format!("cannot resolve bundle path {}: {err}", path.display()))?;
    let meta = std::fs::metadata(&canonical)
        .map_err(|err| format!("cannot read bundle metadata {}: {err}", canonical.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "bundle path is not a regular file: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Maps a [`BundleError`] from [`read_bundle`] to a stable category string.
fn bundle_read_error_category(err: &BundleError) -> &'static str {
    match err {
        BundleError::Io(_) => "io_error",
        BundleError::InvalidArchive(_) => "invalid_archive",
        BundleError::InvalidCompression(_) => "invalid_compression",
        BundleError::InvalidManifest(_) => "invalid_manifest",
        BundleError::UnsupportedAgefVersion(_) => "unsupported_agef_version",
        BundleError::UnsupportedHashAlgorithm(_) => "unsupported_hash_algorithm",
        BundleError::MalformedFraming(_) => "malformed_framing",
        BundleError::FrameTooLarge(_) => "frame_too_large",
        BundleError::NonCanonicalCbor => "non_canonical_cbor",
        BundleError::UnknownEventKind(_) => "unknown_event_kind",
        BundleError::UnknownAttemptStatus(_) => "unknown_attempt_status",
        BundleError::MissingObject(_) => "missing_object",
        BundleError::ObjectHashMismatch(_) => "object_hash_mismatch",
        BundleError::HeadMismatch { .. } => "head_mismatch",
        BundleError::UnknownBundleFile(_) => "unknown_bundle_file",
    }
}

/// Exit code for a [`BundleError`] encountered while reading a bundle.
fn bundle_read_exit_code(err: &BundleError) -> u8 {
    match err {
        BundleError::Io(_) => 3,
        _ => 1,
    }
}

/// Converts a fail-soft [`BundleVerificationReport`] into JSON report violations.
fn report_violations(report: &BundleVerificationReport) -> Vec<ReportViolation> {
    report.violations.iter().map(report_violation).collect()
}

/// Maps one library violation into the CLI-compatible JSON shape.
fn report_violation(v: &BundleViolation) -> ReportViolation {
    ReportViolation {
        category: v.category().to_owned(),
        event_hash: v.event_hash_hex(),
        object_hash: v.object_hash_hex(),
        message: v.message(),
    }
}

/// Builds the machine-readable verification report for a parsed bundle.
fn build_verify_report(
    bundle_path: &Path,
    contents: &BundleContents,
    verification: &BundleVerificationReport,
    sig_report: &SignatureVerificationReport,
    require_signature: bool,
) -> BundleVerifyReportV1 {
    let integrity_ok = verification.is_clean();
    let signatures_ok =
        !sig_report.any_invalid() && (!require_signature || sig_report.any_verified());
    BundleVerifyReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: contents.manifest.agef_version.clone(),
        bundle_path: bundle_path.display().to_string(),
        session_id: contents.manifest.session.id.clone(),
        events_in_bundle: contents.events.len() as u64,
        objects_in_bundle: contents.objects.len() as u64,
        passed: integrity_ok && signatures_ok,
        violations: report_violations(verification),
        signatures: signature_reports(sig_report),
    }
}

/// Prints a verification report as JSON.
fn print_json_report(report: &BundleVerifyReportV1) -> io::Result<()> {
    let json = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
    println!("{json}");
    Ok(())
}

/// Prints an infrastructure/read error as JSON.
fn print_json_infra_error(category: &str, error: String) -> io::Result<()> {
    let body = VerifyInfraErrorV1 {
        tool: "agef-verify",
        tool_version: env!("CARGO_PKG_VERSION").to_owned(),
        error,
        category: category.to_owned(),
    };
    let json = serde_json::to_string_pretty(&body).map_err(io::Error::other)?;
    println!("{json}");
    Ok(())
}

/// Emits a human-readable verification outcome.
fn print_human_report(
    bundle_path: &Path,
    contents: &BundleContents,
    report: &BundleVerifyReportV1,
) {
    let bundle_disp = bundle_path.display();
    if report.passed {
        eprintln!("verified bundle: {bundle_disp}");
    } else {
        eprintln!("bundle verification FAILED: {bundle_disp}");
    }
    eprintln!("  session_id: {}", contents.manifest.session.id);
    eprintln!("  events: {}", contents.events.len());
    eprintln!("  objects: {}", contents.objects.len());
    if !report.violations.is_empty() {
        eprintln!("  integrity violations:");
        for v in &report.violations {
            eprintln!("    - [{}] {}", v.category, v.message);
        }
    }
    if report.signatures.is_empty() {
        eprintln!("  signatures: none");
    } else {
        eprintln!("  signatures:");
        for s in &report.signatures {
            eprintln!("    - {} [{}] key_id={}", s.outcome, s.scheme, s.key_id);
        }
    }
}

/// Emits a human-readable infrastructure error.
fn print_human_infra_error(message: &str) {
    eprintln!("agef-verify: {message}");
}

/// Reads and verifies one bundle file.
fn run_verify(
    bundle_path: PathBuf,
    format: OutputFormat,
    allow_extra_files: bool,
    verify_keys: Vec<PathBuf>,
    require_signature: bool,
) -> ExitCode {
    let json = matches!(format, OutputFormat::Json);
    let bundle = match validated_bundle_path(bundle_path.as_path()) {
        Ok(path) => path,
        Err(msg) => {
            if json {
                let _ = print_json_infra_error("io_error", msg.clone());
            } else {
                print_human_infra_error(&msg);
            }
            return ExitCode::from(3);
        }
    };

    let mut file = match File::open(&bundle) {
        Ok(f) => f,
        Err(err) => {
            let msg = format!("cannot open bundle {}: {err}", bundle.display());
            if json {
                let _ = print_json_infra_error("io_error", msg.clone());
            } else {
                print_human_infra_error(&msg);
            }
            return ExitCode::from(3);
        }
    };

    let options = ReadBundleOptions {
        allow_extra_files,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };

    let contents = match read_bundle(&mut file, &options) {
        Ok(c) => c,
        Err(err) => {
            let msg = err.to_string();
            let category = bundle_read_error_category(&err);
            let code = bundle_read_exit_code(&err);
            if json {
                let _ = print_json_infra_error(category, msg);
            } else {
                print_human_infra_error(&msg);
            }
            return ExitCode::from(code);
        }
    };

    let verification = verify_bundle(&contents);

    // Load any trusted public keys (hex → raw 32 bytes) before signature verification.
    let mut trusted_keys = Vec::with_capacity(verify_keys.len());
    for key_path in &verify_keys {
        let parsed = std::fs::read_to_string(key_path)
            .map_err(|err| format!("cannot read --verify-key {}: {err}", key_path.display()))
            .and_then(|hex_str| {
                parse_public_key_hex(&hex_str)
                    .map_err(|err| format!("--verify-key {}: {err}", key_path.display()))
            });
        match parsed {
            Ok(key) => trusted_keys.push(key),
            Err(msg) => {
                if json {
                    let _ = print_json_infra_error("verify_key_error", msg);
                } else {
                    print_human_infra_error(&msg);
                }
                return ExitCode::from(3);
            }
        }
    }
    let sig_report = verify_manifest_signatures(&contents.manifest, &trusted_keys);

    let report = build_verify_report(
        &bundle,
        &contents,
        &verification,
        &sig_report,
        require_signature,
    );

    if json {
        if let Err(err) = print_json_report(&report) {
            eprintln!("agef-verify: failed to render JSON output: {err}");
            return ExitCode::from(3);
        }
    } else {
        print_human_report(&bundle, &contents, &report);
    }

    if report.passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    run_verify(
        cli.bundle,
        cli.format,
        cli.allow_extra_files,
        cli.verify_keys,
        cli.require_signature,
    )
}
