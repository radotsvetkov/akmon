//! Standalone AGEF bundle integrity verifier.
//!
//! Verifies portable `.akmon` bundles using [`akmon_bundle::verify_bundle`] without
//! the Akmon CLI, journal store, or agent runtime. Intended for auditors and CI
//! pipelines that need a minimal, separately distributable check.
//!
//! The report shape and pass/fail policy live in [`akmon_bundle::report`], shared with
//! `akmon bundle verify` so the two binaries cannot silently diverge on when a bundle counts as
//! verified. This binary owns only its CLI surface and I/O (stdout/stderr, exit codes).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_bundle::report::{
    BundleVerifyReportV1, build_verify_report, bundle_read_error_category, bundle_read_exit_code,
    capture_human_line, capture_human_suffix, compute_passed_and_violations, load_operator_key,
    operator_reports, operator_requirements_ok, print_operator_human_block, signature_reports,
};
use akmon_bundle::{
    DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, key_id, otel_capture_info,
    parse_public_key_hex, read_bundle, verify_bundle, verify_manifest_signatures,
    verify_operator_attestations,
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
    /// Trusted operator Ed25519 public key as hex (64 chars), read from a file. Repeatable. When
    /// provided, each `manifest.operator_attestations[]` entry is verified against these keys
    /// (AGEF v0.1.3, decision D-20).
    #[arg(long = "operator-key", value_name = "HEX_FILE")]
    operator_keys: Vec<PathBuf>,
    /// Fail (exit 1) unless at least one operator attestation verifies against an `--operator-key`.
    #[arg(long, default_value_t = false)]
    require_operator: bool,
    /// Trusted operator public key (hex file) that MUST have attested: fail unless THIS specific key
    /// has a verified attestation. Repeatable; implies trusting these keys for verification.
    #[arg(long = "require-operator-key", value_name = "HEX_FILE")]
    require_operator_keys: Vec<PathBuf>,
    /// Fail unless the session captured full message content (rejects metadata-only OTEL imports).
    #[arg(long, value_enum, value_name = "LEVEL")]
    require_capture: Option<RequireCapture>,
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

/// Required capture level for `--require-capture`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RequireCapture {
    /// Require that the session captured full message content.
    Full,
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
fn print_human_report(bundle_path: &Path, report: &BundleVerifyReportV1) {
    let bundle_disp = bundle_path.display();
    let capture_suffix = capture_human_suffix(report.capture.as_ref());
    if report.passed {
        eprintln!("verified bundle (integrity + signature){capture_suffix}: {bundle_disp}");
    } else {
        eprintln!("bundle verification FAILED: {bundle_disp}");
    }
    eprintln!("  session_id: {}", report.session_id);
    eprintln!("  events: {}", report.events_in_bundle);
    eprintln!("  objects: {}", report.objects_in_bundle);
    if let Some(line) = capture_human_line(report.capture.as_ref()) {
        eprintln!("  {line}");
    }
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
    print_operator_human_block(&report.operators, |line| eprintln!("  {line}"));
}

/// Emits a human-readable infrastructure error.
fn print_human_infra_error(message: &str) {
    eprintln!("agef-verify: {message}");
}

/// Reads and verifies one bundle file.
#[allow(clippy::too_many_arguments)]
fn run_verify(
    bundle_path: PathBuf,
    format: OutputFormat,
    allow_extra_files: bool,
    verify_keys: Vec<PathBuf>,
    require_signature: bool,
    operator_keys: Vec<PathBuf>,
    require_operator: bool,
    require_operator_keys: Vec<PathBuf>,
    require_capture: Option<RequireCapture>,
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

    // Load trusted operator keys: the union of --operator-key and --require-operator-key. Each
    // --require-operator-key is ALSO trusted for verification, and its key_id is recorded so we can
    // demand that THAT specific key produced a verified attestation.
    let mut operator_trusted_keys: Vec<Vec<u8>> = Vec::new();
    for key_path in &operator_keys {
        match load_operator_key(key_path) {
            Ok(key) => operator_trusted_keys.push(key),
            Err(msg) => {
                if json {
                    let _ = print_json_infra_error("operator_key_error", msg);
                } else {
                    print_human_infra_error(&msg);
                }
                return ExitCode::from(3);
            }
        }
    }
    let mut required_operator_key_ids: Vec<String> = Vec::new();
    for key_path in &require_operator_keys {
        match load_operator_key(key_path) {
            Ok(key) => {
                required_operator_key_ids.push(key_id(&key));
                operator_trusted_keys.push(key);
            }
            Err(msg) => {
                if json {
                    let _ = print_json_infra_error("operator_key_error", msg);
                } else {
                    print_human_infra_error(&msg);
                }
                return ExitCode::from(3);
            }
        }
    }
    let op_report = verify_operator_attestations(&contents.manifest, &operator_trusted_keys);
    let operator_ok =
        operator_requirements_ok(&op_report, require_operator, &required_operator_key_ids);

    let capture = otel_capture_info(&contents);
    let require_capture_full = matches!(require_capture, Some(RequireCapture::Full));

    let (passed, violations) = compute_passed_and_violations(
        &verification,
        &sig_report,
        require_signature,
        operator_ok,
        capture.as_ref(),
        require_capture_full,
    );
    let report = build_verify_report(
        bundle.display().to_string(),
        &contents,
        passed,
        violations,
        signature_reports(&sig_report),
        operator_reports(&op_report),
        capture.as_ref(),
    );

    if json {
        if let Err(err) = print_json_report(&report) {
            eprintln!("agef-verify: failed to render JSON output: {err}");
            return ExitCode::from(3);
        }
    } else {
        print_human_report(&bundle, &report);
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
        cli.operator_keys,
        cli.require_operator,
        cli.require_operator_keys,
        cli.require_capture,
    )
}
