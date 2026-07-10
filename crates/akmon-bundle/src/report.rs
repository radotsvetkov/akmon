//! Shared bundle-verification report shape and pass/fail policy.
//!
//! `akmon bundle verify` and `agef-verify` are two independent binaries that both decide whether
//! a bundle is trustworthy, and their JSON/human output is what an auditor actually reads. Before
//! this module existed, each binary carried its own copy of the report struct and the
//! pass/fail policy (integrity + signature + operator-attestation + capture requirements); a fix
//! to one copy could silently miss the other, so the same bundle could verify differently
//! depending on which binary ran it. This module is the single implementation both binaries call
//! into, so that cannot happen.

use std::path::Path;

use serde::Serialize;

use crate::archive::BundleContents;
use crate::capture::OtelCaptureInfo;
use crate::error::BundleError;
use crate::signing::{
    OperatorOutcome, OperatorVerificationReport, SignatureOutcome, SignatureVerificationReport,
    parse_public_key_hex,
};
use crate::verify::BundleVerificationReport;

/// Machine-readable bundle verification result, shared by `akmon bundle verify`
/// (`akmon bundle import --verify-only`) and `agef-verify`.
#[derive(Debug, Serialize)]
pub struct BundleVerifyReportV1 {
    /// Verifier crate version.
    pub akmon_version: String,
    /// AGEF specification version declared by the bundle manifest.
    pub agef_version: String,
    /// Path to the bundle file that was verified.
    pub bundle_path: String,
    /// Session UUID from the bundle manifest.
    pub session_id: String,
    /// Number of events decoded from `events.bin`.
    pub events_in_bundle: u64,
    /// Number of objects decoded from `objects/`.
    pub objects_in_bundle: u64,
    /// Holistic verdict: integrity clean, no invalid signatures, no invalid operator
    /// attestations, and (with the relevant `--require-*` flags) at least one signature and/or
    /// operator attestation verified, and any `--require-capture` satisfied.
    pub passed: bool,
    /// Collected integrity violations (empty when structurally clean).
    pub violations: Vec<ReportViolation>,
    /// Per-signature verification results (empty when the bundle is unsigned).
    #[serde(default)]
    pub signatures: Vec<SignatureReport>,
    /// Per-operator-attestation verification results (empty when the bundle is unattributed). NOT
    /// skipped when empty, so an absent operator attestation is observable as `[]`.
    #[serde(default)]
    pub operators: Vec<OperatorReport>,
    /// OTEL-import capture level, or `null` for native (non-OTEL) bundles. A native bundle is
    /// full-fidelity by construction; an OTEL import with `level == "structural"` carries metadata
    /// only (the source telemetry did not capture message content).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureField>,
}

/// OTEL-import capture metadata for JSON output (F1).
#[derive(Debug, Serialize)]
pub struct CaptureField {
    /// Capture level: `full` (message content captured) or `structural` (metadata only).
    pub level: String,
    /// Source OpenTelemetry semantic-conventions version (for example `1.37.0`).
    pub source_semconv: String,
}

/// One signature-verification result for JSON output.
#[derive(Debug, Serialize)]
pub struct SignatureReport {
    /// `key_id` from the manifest entry (hex SHA-256 of the signer public key).
    pub key_id: String,
    /// Signature scheme (`ed25519`).
    pub scheme: String,
    /// Outcome: `verified`, `invalid`, `unverified_no_key`, `unsupported_scheme`, or `malformed`.
    pub outcome: String,
}

/// Stable lowercase outcome string for [`SignatureReport::outcome`].
#[must_use]
pub fn signature_outcome_str(outcome: &SignatureOutcome) -> &'static str {
    match outcome {
        SignatureOutcome::Verified => "verified",
        SignatureOutcome::Invalid => "invalid",
        SignatureOutcome::UnverifiedNoKey => "unverified_no_key",
        SignatureOutcome::UnsupportedScheme => "unsupported_scheme",
        SignatureOutcome::Malformed => "malformed",
    }
}

/// Maps the library signature report into report-compatible JSON entries.
#[must_use]
pub fn signature_reports(report: &SignatureVerificationReport) -> Vec<SignatureReport> {
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
/// Honesty contract (O8): `operator_key_verified` is the only field that attests trust — `true`
/// ONLY when a trusted operator key validated the signed `AGEF-OPERATOR-v1` statement. The
/// identity fields are self-asserted and attacker-controlled; their truth is gated by
/// `operator_key_verified`.
#[derive(Debug, Serialize)]
pub struct OperatorReport {
    /// `key_id` from the manifest entry (hex SHA-256 of the attester public key).
    pub key_id: String,
    /// Signature scheme (`ed25519`).
    pub scheme: String,
    /// Self-asserted operator identifier (signed but attacker-controlled until verified).
    pub operator_id: String,
    /// Self-asserted operator display name.
    pub display_name: String,
    /// Self-asserted operator role.
    pub role: String,
    /// Self-asserted operator organization.
    pub org: String,
    /// RFC3339 timestamp the attestation was produced (unsigned metadata).
    pub created_at: String,
    /// Outcome: `verified`, `invalid`, `unverified_no_key`, `unsupported_scheme`, or `malformed`.
    pub outcome: String,
    /// `true` ONLY when `outcome == verified` (a trusted operator key validated the attestation).
    pub operator_key_verified: bool,
}

/// Stable lowercase outcome string for [`OperatorReport::outcome`].
#[must_use]
pub fn operator_outcome_str(outcome: &OperatorOutcome) -> &'static str {
    match outcome {
        OperatorOutcome::Verified => "verified",
        OperatorOutcome::Invalid => "invalid",
        OperatorOutcome::UnverifiedNoKey => "unverified_no_key",
        OperatorOutcome::UnsupportedScheme => "unsupported_scheme",
        OperatorOutcome::Malformed => "malformed",
    }
}

/// Maps the library operator report into report-compatible JSON entries (parallels
/// [`signature_reports`]).
#[must_use]
pub fn operator_reports(report: &OperatorVerificationReport) -> Vec<OperatorReport> {
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
/// attacker-controlled operator identity field before printing it in the human report (O8).
/// Without this a crafted `operator_id` containing a newline or terminal escape could spoof the
/// surrounding report lines. Renders such bytes as `\xNN` so the value stays on one visible line.
#[must_use]
pub fn sanitize_operator_field(value: &str) -> String {
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
pub fn load_operator_key(key_path: &Path) -> Result<Vec<u8>, String> {
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
#[must_use]
pub fn operator_requirements_ok(
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

/// Maps a [`BundleError`] from [`crate::read_bundle`] to a stable category string.
#[must_use]
pub fn bundle_read_error_category(err: &BundleError) -> &'static str {
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
        BundleError::BundleTooLarge(_) => "bundle_too_large",
    }
}

/// Exit code for a [`BundleError`] encountered while reading a bundle.
#[must_use]
pub fn bundle_read_exit_code(err: &BundleError) -> u8 {
    match err {
        BundleError::Io(_) => 3,
        _ => 1,
    }
}

/// One bundle verification violation for JSON output.
#[derive(Debug, Serialize)]
pub struct ReportViolation {
    /// Stable category identifier.
    pub category: String,
    /// Event content hash in hex when applicable.
    pub event_hash: Option<String>,
    /// Object hash in hex when applicable.
    pub object_hash: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// Converts a fail-soft [`BundleVerificationReport`] into JSON report violations.
#[must_use]
pub fn report_violations(report: &BundleVerificationReport) -> Vec<ReportViolation> {
    report
        .violations
        .iter()
        .map(|v| ReportViolation {
            category: v.category().to_owned(),
            event_hash: v.event_hash_hex(),
            object_hash: v.object_hash_hex(),
            message: v.message(),
        })
        .collect()
}

/// Category string used for the synthetic violation emitted when `--require-capture full` fails.
pub const CAPTURE_REQUIREMENT_CATEGORY: &str = "capture_requirement_unmet";

/// Whether an OTEL import satisfies `--require-capture full`.
///
/// Native bundles (`capture == None`) are full-fidelity and always pass. An OTEL import passes
/// only when its `capture_level` is exactly `"full"`. Returns `None` (no requirement) when
/// `require_capture_full` is `false`.
#[must_use]
pub fn capture_requirement_unmet(
    require_capture_full: bool,
    capture: Option<&OtelCaptureInfo>,
) -> Option<String> {
    if !require_capture_full {
        return None;
    }
    match capture {
        None => None,
        Some(info) if info.capture_level == "full" => None,
        Some(info) => Some(format!(
            "--require-capture full: session capture level is '{}', not 'full' (metadata-only OTEL import; source telemetry did not capture message content)",
            info.capture_level
        )),
    }
}

/// Short suffix appended to the success headline so a structural import never reads as a bare
/// "verified bundle". Native bundles (`None`) and full OTEL imports add nothing.
#[must_use]
pub fn capture_human_suffix(capture: Option<&CaptureField>) -> String {
    match capture {
        Some(c) if c.level == "structural" => " — capture: STRUCTURAL (metadata only)".to_owned(),
        _ => String::new(),
    }
}

/// Prominent capture line for the human report body, or `None` for native (non-OTEL) bundles.
#[must_use]
pub fn capture_human_line(capture: Option<&CaptureField>) -> Option<String> {
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

/// Computes the holistic pass/fail verdict and the violation list (integrity violations plus, if
/// applicable, the synthetic `--require-capture` violation) for a verified bundle.
///
/// This is the trust decision itself: shared so `akmon bundle verify` and `agef-verify` cannot
/// diverge on when a bundle counts as passed.
#[must_use]
pub fn compute_passed_and_violations(
    verification: &BundleVerificationReport,
    sig_report: &SignatureVerificationReport,
    require_signature: bool,
    operator_ok: bool,
    capture: Option<&OtelCaptureInfo>,
    require_capture_full: bool,
) -> (bool, Vec<ReportViolation>) {
    let integrity_ok = verification.is_clean();
    let signatures_ok =
        !sig_report.any_invalid() && (!require_signature || sig_report.any_verified());
    let capture_unmet = capture_requirement_unmet(require_capture_full, capture);
    let mut violations = report_violations(verification);
    if let Some(reason) = &capture_unmet {
        violations.push(ReportViolation {
            category: CAPTURE_REQUIREMENT_CATEGORY.to_owned(),
            event_hash: None,
            object_hash: None,
            message: reason.clone(),
        });
    }
    let passed = integrity_ok && signatures_ok && operator_ok && capture_unmet.is_none();
    (passed, violations)
}

/// Assembles the final [`BundleVerifyReportV1`] from already-computed pieces.
///
/// Takes `passed`/`violations`/`signatures`/`operators` as inputs (rather than computing them)
/// so callers that skip straight to a failure report (for example an import-time integrity
/// failure, with no signature or operator checks run) can still produce a well-formed report.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn build_verify_report(
    bundle_path_display: String,
    contents: &BundleContents,
    passed: bool,
    violations: Vec<ReportViolation>,
    signatures: Vec<SignatureReport>,
    operators: Vec<OperatorReport>,
    capture: Option<&OtelCaptureInfo>,
) -> BundleVerifyReportV1 {
    BundleVerifyReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: contents.manifest.agef_version.clone(),
        bundle_path: bundle_path_display,
        session_id: contents.manifest.session.id.clone(),
        events_in_bundle: contents.events.len() as u64,
        objects_in_bundle: contents.objects.len() as u64,
        passed,
        violations,
        signatures,
        operators,
        capture: capture.map(|info| CaptureField {
            level: info.capture_level.clone(),
            source_semconv: info.source_semconv.clone(),
        }),
    }
}

/// Emits the human-readable operator-attestation block (decision D-20, honesty contract O8).
///
/// "verified" attaches to the KEY, never the self-asserted identity string. Self-asserted fields
/// are attacker-controlled and are sanitized ([`sanitize_operator_field`]) before printing so they
/// cannot spoof the surrounding report. Each line is written via `emit` so both `akmon` and
/// `agef-verify` can share the same wording and route it to their own output stream.
pub fn print_operator_human_block(operators: &[OperatorReport], mut emit: impl FnMut(String)) {
    if operators.is_empty() {
        emit("operator: none (unattributed)".to_owned());
        return;
    }
    emit("operators:".to_owned());
    for op in operators {
        let oid = sanitize_operator_field(&op.operator_id);
        let role = sanitize_operator_field(&op.role);
        let org = sanitize_operator_field(&op.org);
        let key_id = &op.key_id;
        let scheme = &op.scheme;
        let line = match op.outcome.as_str() {
            "verified" => format!(
                "  - verified by operator key key_id={key_id} -- self-asserted operator_id=\"{oid}\" role=\"{role}\" org=\"{org}\""
            ),
            "unverified_no_key" => format!(
                "  - present but UNVERIFIED (no trusted operator key) -- self-asserted operator_id=\"{oid}\" (do not trust this name)"
            ),
            "invalid" => format!(
                "  - INVALID signature for trusted operator key key_id={key_id} (tampered identity/head or corrupt signature; HARD FAIL)"
            ),
            other => format!(
                "  - {other} [{scheme}] key_id={key_id} -- self-asserted operator_id=\"{oid}\" (do not trust this name)"
            ),
        };
        emit(line);
    }
}
