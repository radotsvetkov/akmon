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
    DEFAULT_MAX_EVENT_FRAME_LEN, OperatorOutcome, OperatorVerificationReport, OtelCaptureInfo,
    ReadBundleOptions, SignatureOutcome, SignatureVerificationReport, key_id, otel_capture_info,
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
    /// Per-operator-attestation verification results (empty when the bundle is unattributed). NOT
    /// skipped when empty, so an absent operator attestation is observable as `[]`.
    #[serde(default)]
    operators: Vec<OperatorReport>,
    /// OTEL-import capture level, or `null` for native (non-OTEL) bundles. A native bundle is
    /// full-fidelity by construction; an OTEL import with `level == "structural"` carries metadata
    /// only (the source telemetry did not capture message content).
    #[serde(skip_serializing_if = "Option::is_none")]
    capture: Option<CaptureField>,
}

/// OTEL-import capture metadata for JSON output (F1).
#[derive(Debug, Serialize)]
struct CaptureField {
    /// Capture level: `full` (message content captured) or `structural` (metadata only).
    level: String,
    /// Source OpenTelemetry semantic-conventions version (for example `1.37.0`).
    source_semconv: String,
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

/// One operator-attestation verification result for JSON output (D-20).
///
/// Honesty contract (O8): `operator_key_verified` is the only field that attests trust — `true` ONLY
/// when a trusted operator key validated the signed `AGEF-OPERATOR-v1` statement. The identity
/// fields are self-asserted and attacker-controlled; their truth is gated by `operator_key_verified`.
#[derive(Debug, Serialize)]
struct OperatorReport {
    /// `key_id` from the manifest entry (hex SHA-256 of the attester public key).
    key_id: String,
    /// Signature scheme (`ed25519`).
    scheme: String,
    /// Self-asserted operator identifier (signed but attacker-controlled until verified).
    operator_id: String,
    /// Self-asserted operator display name.
    display_name: String,
    /// Self-asserted operator role.
    role: String,
    /// Self-asserted operator organization.
    org: String,
    /// RFC3339 timestamp the attestation was produced (unsigned metadata).
    created_at: String,
    /// Outcome: `verified`, `invalid`, `unverified_no_key`, `unsupported_scheme`, or `malformed`.
    outcome: String,
    /// `true` ONLY when `outcome == verified` (a trusted operator key validated the attestation).
    operator_key_verified: bool,
}

/// Stable lowercase outcome string for [`OperatorReport::outcome`].
fn operator_outcome_str(outcome: &OperatorOutcome) -> &'static str {
    match outcome {
        OperatorOutcome::Verified => "verified",
        OperatorOutcome::Invalid => "invalid",
        OperatorOutcome::UnverifiedNoKey => "unverified_no_key",
        OperatorOutcome::UnsupportedScheme => "unsupported_scheme",
        OperatorOutcome::Malformed => "malformed",
    }
}

/// Maps the library operator report into CLI-compatible JSON entries (parallels
/// [`signature_reports`]).
fn operator_reports(report: &OperatorVerificationReport) -> Vec<OperatorReport> {
    report
        .checks
        .iter()
        .map(|c| OperatorReport {
            key_id: c.key_id.clone(),
            scheme: c.scheme.clone(),
            operator_id: c.operator_id.clone(),
            display_name: c.display_name.clone(),
            role: c.role.clone(),
            org: c.org.clone(),
            created_at: c.created_at.clone(),
            outcome: operator_outcome_str(&c.outcome).to_owned(),
            operator_key_verified: c.outcome == OperatorOutcome::Verified,
        })
        .collect()
}

/// Escapes control characters (anything below `0x20`, plus `0x7f`) in a self-asserted,
/// attacker-controlled operator identity field before printing it in the human report (O8). Without
/// this a crafted `operator_id` containing a newline or terminal escape could spoof the surrounding
/// report lines. Renders such bytes as `\xNN` so the value stays on one visible line.
fn sanitize_operator_field(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if (ch as u32) < 0x20 || ch == '\u{7f}' {
            out.push_str(&format!("\\x{:02x}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Loads a trusted operator public key (hex file -> raw 32 bytes) for `--operator-key` /
/// `--require-operator-key`. Mirrors the `--verify-key` loader's error contract.
fn load_operator_key(key_path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read_to_string(key_path)
        .map_err(|err| format!("cannot read --operator-key {}: {err}", key_path.display()))
        .and_then(|hex_str| {
            parse_public_key_hex(&hex_str)
                .map_err(|err| format!("--operator-key {}: {err}", key_path.display()))
        })
}

/// Computes whether the operator-attestation requirements are satisfied (decision D-20).
///
/// An invalid attestation against a trusted key is ALWAYS a hard failure (via `any_invalid()`),
/// mirroring head signatures, even without `--require-operator`. `--require-operator` additionally
/// demands at least one verified attestation, and each `--require-operator-key` demands that THAT
/// specific key (`key_id`) has a verified attestation.
fn operator_requirements_ok(
    op_report: &OperatorVerificationReport,
    require_operator: bool,
    required_operator_key_ids: &[String],
) -> bool {
    if op_report.any_invalid() {
        return false;
    }
    if require_operator && !op_report.any_verified() {
        return false;
    }
    required_operator_key_ids.iter().all(|required_kid| {
        op_report
            .checks
            .iter()
            .any(|c| c.outcome == OperatorOutcome::Verified && &c.key_id == required_kid)
    })
}

/// Emits the human-readable operator-attestation block (decision D-20, honesty contract O8).
///
/// "verified" attaches to the KEY, never the self-asserted identity string. Self-asserted fields are
/// sanitized ([`sanitize_operator_field`]) before printing so they cannot spoof the surrounding
/// report.
fn print_operator_human_block(operators: &[OperatorReport]) {
    if operators.is_empty() {
        eprintln!("  operator: none (unattributed)");
        return;
    }
    eprintln!("  operators:");
    for op in operators {
        let oid = sanitize_operator_field(&op.operator_id);
        let role = sanitize_operator_field(&op.role);
        let org = sanitize_operator_field(&op.org);
        let key_id = &op.key_id;
        let scheme = &op.scheme;
        match op.outcome.as_str() {
            "verified" => eprintln!(
                "    - verified by operator key key_id={key_id} -- self-asserted operator_id=\"{oid}\" role=\"{role}\" org=\"{org}\""
            ),
            "unverified_no_key" => eprintln!(
                "    - present but UNVERIFIED (no trusted operator key) -- self-asserted operator_id=\"{oid}\" (do not trust this name)"
            ),
            "invalid" => eprintln!(
                "    - INVALID signature for trusted operator key key_id={key_id} (tampered identity/head or corrupt signature; HARD FAIL)"
            ),
            other => eprintln!(
                "    - {other} [{scheme}] key_id={key_id} -- self-asserted operator_id=\"{oid}\" (do not trust this name)"
            ),
        }
    }
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

/// Category string used for the synthetic violation emitted when `--require-capture full` fails.
const CAPTURE_REQUIREMENT_CATEGORY: &str = "capture_requirement_unmet";

/// Whether an OTEL import satisfies `--require-capture full`.
///
/// Native bundles (`capture == None`) are full-fidelity and always pass. An OTEL import passes only
/// when its `capture_level` is exactly `"full"`. Returns `None` (no requirement) when
/// `require_capture` is unset.
fn capture_requirement_unmet(
    require_capture: Option<RequireCapture>,
    capture: Option<&OtelCaptureInfo>,
) -> Option<String> {
    match require_capture {
        None => None,
        Some(RequireCapture::Full) => match capture {
            None => None,
            Some(info) if info.capture_level == "full" => None,
            Some(info) => Some(format!(
                "--require-capture full: session capture level is '{}', not 'full' (metadata-only OTEL import; source telemetry did not capture message content)",
                info.capture_level
            )),
        },
    }
}

/// Builds the machine-readable verification report for a parsed bundle.
#[allow(clippy::too_many_arguments)]
fn build_verify_report(
    bundle_path: &Path,
    contents: &BundleContents,
    verification: &BundleVerificationReport,
    sig_report: &SignatureVerificationReport,
    require_signature: bool,
    op_report: &OperatorVerificationReport,
    operator_ok: bool,
    capture: Option<&OtelCaptureInfo>,
    require_capture: Option<RequireCapture>,
) -> BundleVerifyReportV1 {
    let integrity_ok = verification.is_clean();
    let signatures_ok =
        !sig_report.any_invalid() && (!require_signature || sig_report.any_verified());
    let capture_unmet = capture_requirement_unmet(require_capture, capture);
    let mut violations = report_violations(verification);
    if let Some(reason) = &capture_unmet {
        violations.push(ReportViolation {
            category: CAPTURE_REQUIREMENT_CATEGORY.to_owned(),
            event_hash: None,
            object_hash: None,
            message: reason.clone(),
        });
    }
    BundleVerifyReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: contents.manifest.agef_version.clone(),
        bundle_path: bundle_path.display().to_string(),
        session_id: contents.manifest.session.id.clone(),
        events_in_bundle: contents.events.len() as u64,
        objects_in_bundle: contents.objects.len() as u64,
        passed: integrity_ok && signatures_ok && operator_ok && capture_unmet.is_none(),
        violations,
        signatures: signature_reports(sig_report),
        operators: operator_reports(op_report),
        capture: capture.map(|info| CaptureField {
            level: info.capture_level.clone(),
            source_semconv: info.source_semconv.clone(),
        }),
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

/// Short suffix appended to the success headline so a structural import never reads as a bare
/// "verified bundle". Native bundles (`None`) and full OTEL imports add nothing.
fn capture_human_suffix(capture: Option<&CaptureField>) -> String {
    match capture {
        Some(c) if c.level == "structural" => " — capture: STRUCTURAL (metadata only)".to_owned(),
        _ => String::new(),
    }
}

/// Prominent capture line for the human report body, or `None` for native (non-OTEL) bundles.
fn capture_human_line(capture: Option<&CaptureField>) -> Option<String> {
    let c = capture?;
    match c.level.as_str() {
        "structural" => Some(format!(
            "capture: STRUCTURAL (metadata only — message content was NOT captured by the source telemetry; source semconv {})",
            c.source_semconv
        )),
        "full" => Some("capture: FULL (OTEL import; message content captured)".to_owned()),
        other => Some(format!(
            "capture: {} (OTEL import; source semconv {})",
            other.to_uppercase(),
            c.source_semconv
        )),
    }
}

/// Emits a human-readable verification outcome.
fn print_human_report(
    bundle_path: &Path,
    contents: &BundleContents,
    report: &BundleVerifyReportV1,
) {
    let bundle_disp = bundle_path.display();
    let capture_suffix = capture_human_suffix(report.capture.as_ref());
    if report.passed {
        eprintln!("verified bundle (integrity + signature){capture_suffix}: {bundle_disp}");
    } else {
        eprintln!("bundle verification FAILED: {bundle_disp}");
    }
    eprintln!("  session_id: {}", contents.manifest.session.id);
    eprintln!("  events: {}", contents.events.len());
    eprintln!("  objects: {}", contents.objects.len());
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
    print_operator_human_block(&report.operators);
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

    let report = build_verify_report(
        &bundle,
        &contents,
        &verification,
        &sig_report,
        require_signature,
        &op_report,
        operator_ok,
        capture.as_ref(),
        require_capture,
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
        cli.operator_keys,
        cli.require_operator,
        cli.require_operator_keys,
        cli.require_capture,
    )
}
