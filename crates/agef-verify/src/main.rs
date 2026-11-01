//! Standalone AGEF bundle integrity verifier.
//!
//! Verifies portable `.akmon` bundles using [`akmon_bundle::verify_bundle`] without
//! the Akmon CLI, journal store, or agent runtime. Intended for auditors and CI
//! pipelines that need a minimal, separately distributable check.
//!
//! The report shape, pass/fail policy, and the human/Markdown renderings live in
//! [`akmon_bundle::report`] / [`akmon_bundle::render`], shared with `akmon bundle verify` so the two
//! binaries cannot silently diverge on when a bundle counts as verified or on what they print.
//! This binary owns only its CLI surface and I/O (stdout/stderr, exit codes).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_bundle::report::{
    ToolErrorV1, VerifyPolicy, bundle_read_error_category, bundle_read_exit_code, evaluate_bundle,
};
use akmon_bundle::{
    DEFAULT_MAX_BUNDLE_DECODED_BYTES, DEFAULT_MAX_EVENT_FRAME_LEN, ReadBundleOptions, human_lines,
    markdown, read_bundle_limited, sanitize_display,
};
use clap::{Parser, ValueEnum};

const TOOL: &str = "agef-verify";

/// Command-line interface for `agef-verify`.
#[derive(Debug, Parser)]
#[command(
    name = "agef-verify",
    version,
    about = "Verify an AGEF .akmon bundle without the Akmon CLI",
    after_help = "Exit codes:\n  \
0 — verification passed (without --require-signature this means INTEGRITY ONLY; check\n      \
signature_verified in --format json)\n  \
1 — verification failed, including a malformed, corrupt, truncated, or oversized archive\n  \
2 — usage error (invalid arguments)\n  \
3 — I/O or environment error (bundle or key file missing/unreadable, invalid key file)"
)]
struct Cli {
    /// Path to a `.akmon` bundle file.
    bundle: PathBuf,
    /// Output format: human (stderr), json (stdout), or markdown (auditor report on stdout).
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
    /// Maximum total decompressed size of the bundle in bytes (decompression-bomb guard).
    #[arg(long, value_name = "BYTES", default_value_t = DEFAULT_MAX_BUNDLE_DECODED_BYTES)]
    max_decoded_bytes: u64,
}

/// Output mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    /// Plain-text report on stderr.
    #[default]
    Human,
    /// `BundleVerifyReportV1`-compatible JSON on stdout.
    Json,
    /// Plain-language Markdown auditor report on stdout.
    Markdown,
}

/// Required capture level for `--require-capture`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RequireCapture {
    /// Require that the session captured full message content.
    Full,
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

/// Reports an infrastructure/read error in the requested format and returns `code`.
fn fail(format: OutputFormat, category: &str, message: String, code: u8) -> ExitCode {
    if format == OutputFormat::Json {
        ToolErrorV1::new(TOOL, category, message).print_json();
    } else {
        eprintln!("{TOOL}: {}", sanitize_display(&message));
    }
    ExitCode::from(code)
}

/// Reads and verifies one bundle file.
fn run_verify(cli: Cli) -> ExitCode {
    let format = cli.format;
    let bundle = match validated_bundle_path(cli.bundle.as_path()) {
        Ok(path) => path,
        Err(msg) => return fail(format, "io_error", msg, 3),
    };

    let policy = match VerifyPolicy::load(
        &cli.verify_keys,
        cli.require_signature,
        &cli.operator_keys,
        cli.require_operator,
        &cli.require_operator_keys,
        matches!(cli.require_capture, Some(RequireCapture::Full)),
    ) {
        Ok(policy) => policy,
        Err(err) => return fail(format, err.category, err.message, 3),
    };

    let mut file = match File::open(&bundle) {
        Ok(f) => f,
        Err(err) => {
            let msg = format!("cannot open bundle {}: {err}", bundle.display());
            return fail(format, "io_error", msg, 3);
        }
    };
    let options = ReadBundleOptions {
        allow_extra_files: cli.allow_extra_files,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let contents = match read_bundle_limited(&mut file, &options, cli.max_decoded_bytes) {
        Ok(c) => c,
        Err(err) => {
            let category = bundle_read_error_category(&err);
            let code = bundle_read_exit_code(&err);
            return fail(format, category, err.to_string(), code);
        }
    };

    let report = evaluate_bundle(bundle.display().to_string(), &contents, &policy);

    match format {
        OutputFormat::Json => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("{TOOL}: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        },
        OutputFormat::Markdown => print!("{}", markdown(&report)),
        OutputFormat::Human => {
            for line in human_lines(&report) {
                eprintln!("{line}");
            }
        }
    }

    if report.passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    run_verify(Cli::parse())
}
