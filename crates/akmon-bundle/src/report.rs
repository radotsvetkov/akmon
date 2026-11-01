//! Shared bundle-verification report shape and pass/fail policy.
//!
//! `akmon bundle verify` and `agef-verify` are two independent binaries that both decide whether
//! a bundle is trustworthy, and their JSON/human/Markdown output is what an auditor actually reads.
//! Before this module existed, each binary carried its own copy of the report struct and the
//! pass/fail policy (integrity + signature + operator-attestation + capture requirements); a fix
//! to one copy could silently miss the other, so the same bundle could verify differently
//! depending on which binary ran it. This module is the single implementation both binaries call
//! into ([`VerifyPolicy::load`] + [`evaluate_bundle`]), so that cannot happen. Rendering lives in
//! [`crate::render`].

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::archive::BundleContents;
use crate::capture::{OtelCaptureInfo, otel_capture_info};
use crate::error::BundleError;
use crate::sanitize::sanitize_display;
use crate::signing::{
    OperatorOutcome, OperatorVerificationReport, SignatureOutcome, SignatureVerificationReport,
    key_id, parse_public_key_hex, verify_manifest_signatures, verify_operator_attestations,
};
use crate::verify::{BundleVerificationReport, verify_bundle};

pub use crate::render::{capture_human_line, capture_human_suffix, print_operator_human_block};

/// Machine-readable bundle verification result, shared by `akmon bundle verify`
/// (`akmon bundle import --verify-only`) and `agef-verify`.
///
/// Fields after `capture` were added in v3.0.0; they are additive, so V1 consumers keep working.
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
    ///
    /// Without `--require-signature`, `passed == true` means INTEGRITY ONLY: read
    /// [`Self::signature_verified`] to learn whether a trusted key authenticated the head.
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
    /// only (the source telemetry did not capture message content), and `level == "partial"`
    /// carries message content for only some provider calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureField>,
    /// `true` when the AGEF integrity checks (hash chain, objects, head, counts) found nothing.
    pub integrity_ok: bool,
    /// `true` ONLY when integrity is OK AND at least one head signature verified against a trusted
    /// `--verify-key`. Always `false` when [`Self::integrity_ok`] is `false` (a signature over a head
    /// the contents do not hash to authenticates nothing); see `signatures` for per-entry outcomes.
    pub signature_verified: bool,
    /// `true` ONLY when integrity is OK AND at least one operator attestation verified against a
    /// trusted operator key. Always `false` when [`Self::integrity_ok`] is `false`. The operator
    /// identity strings stay self-asserted; it is the KEY that is verified.
    pub operator_key_verified: bool,
    /// Manifest hash algorithm (`sha256` or `blake3`).
    pub hash_algorithm: String,
    /// Manifest session head (the digest every signature and attestation covers).
    pub head: String,
    /// First and last event timestamps (inside the hash chain, as recorded by the producer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_window: Option<SessionWindow>,
    /// Manifest metadata that NO signature covers: anyone holding the file can edit it.
    pub unsigned_metadata: UnsignedMetadata,
    /// Non-fatal findings (for example manifest timestamps that disagree with the events).
    pub warnings: Vec<ReportViolation>,
}

/// First/last event timestamps, RFC 3339 UTC. These come from the events themselves, so they are
/// covered by integrity (and by any signature over the head) — but they are the producer's own
/// clock readings, not a trusted timestamp.
#[derive(Debug, Clone, Serialize)]
pub struct SessionWindow {
    /// `emitted_at` of the first event.
    pub first_event_at: String,
    /// `emitted_at` of the last event.
    pub last_event_at: String,
}

/// Manifest fields that are not part of any signed statement (AGEF-SIG-v1 covers only
/// `agef_version`, `hash_algorithm`, `session_id`, and `head`).
#[derive(Debug, Clone, Serialize)]
pub struct UnsignedMetadata {
    /// `manifest.session.created_at`.
    pub created_at: String,
    /// `manifest.session.ended_at`.
    pub ended_at: String,
    /// `manifest.producer.name`.
    pub producer_name: String,
    /// `manifest.producer.version`.
    pub producer_version: String,
}

/// OTEL-import capture metadata for JSON output (F1).
#[derive(Debug, Serialize)]
pub struct CaptureField {
    /// Capture level: `full` (message content captured for every provider call), `partial`
    /// (content captured for some provider calls only), or `structural` (metadata only).
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

/// Escapes an attacker-controlled manifest string for terminal display (O8). Kept for API
/// compatibility; delegates to the one shared escaper, [`sanitize_display`].
#[must_use]
pub fn sanitize_operator_field(value: &str) -> String {
    sanitize_display(value)
}

/// Loads a trusted public key (hex file -> raw 32 bytes). `flag` names the CLI flag in errors.
fn load_public_key_file(key_path: &Path, flag: &str) -> Result<Vec<u8>, String> {
    std::fs::read_to_string(key_path)
        .map_err(|err| format!("cannot read {flag} {}: {err}", key_path.display()))
        .and_then(|hex_str| {
            parse_public_key_hex(&hex_str)
                .map_err(|err| format!("{flag} {}: {err}", key_path.display()))
        })
}

/// Loads a trusted signer public key (hex file -> raw 32 bytes) for `--verify-key`.
pub fn load_verify_key(key_path: &Path) -> Result<Vec<u8>, String> {
    load_public_key_file(key_path, "--verify-key")
}

/// Loads a trusted operator public key (hex file -> raw 32 bytes) for `--operator-key` /
/// `--require-operator-key`. Mirrors the `--verify-key` loader's error contract.
pub fn load_operator_key(key_path: &Path) -> Result<Vec<u8>, String> {
    load_public_key_file(key_path, "--operator-key")
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

/// Exit code for a [`BundleError`] encountered while reading a bundle: 3 for an I/O failure, 1 for
/// everything else (a malformed, corrupt, truncated, or oversized archive is a failed verification).
#[must_use]
pub fn bundle_read_exit_code(err: &BundleError) -> u8 {
    match err {
        BundleError::Io(_) => 3,
        _ => 1,
    }
}

/// One shared JSON error shape for every bundle command and both verifier binaries (TC-13).
///
/// Emitted on stdout whenever a command cannot produce its normal report (unreadable file,
/// malformed archive, bad key, refused operation). `passed` is always `false`, so a consumer can
/// read one field on every path, success report or error.
#[derive(Debug, Serialize)]
pub struct ToolErrorV1 {
    /// Producer tool name (`akmon` or `agef-verify`).
    pub tool: String,
    /// Producer crate version.
    pub akmon_version: String,
    /// Always `false`.
    pub passed: bool,
    /// Human-readable error description.
    pub error: String,
    /// Stable error category for automation.
    pub category: String,
    /// Target session UUID when `category` is `session_id_collision` (import only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colliding_session_id: Option<String>,
}

impl ToolErrorV1 {
    /// Builds an error body for `tool` with the given category and message.
    #[must_use]
    pub fn new(tool: &str, category: &str, error: String) -> Self {
        Self {
            tool: tool.to_owned(),
            akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
            passed: false,
            error,
            category: category.to_owned(),
            colliding_session_id: None,
        }
    }

    /// Prints the error as pretty JSON on stdout (best effort; serialization cannot fail for this
    /// plain-string shape, and a failure is silently dropped rather than panicking).
    pub fn print_json(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            println!("{json}");
        }
    }
}

/// One bundle verification violation (or warning) for JSON output.
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

/// Warning category: `manifest.session.created_at` disagrees with the first event timestamp.
pub const CREATED_AT_MISMATCH_CATEGORY: &str = "manifest_created_at_mismatch";

/// Warning category: `manifest.session.ended_at` disagrees with the last event timestamp.
pub const ENDED_AT_MISMATCH_CATEGORY: &str = "manifest_ended_at_mismatch";

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
        Some(info) => {
            let why = match info.capture_level.as_str() {
                "structural" => {
                    "metadata-only OTEL import; source telemetry did not capture message content"
                }
                "partial" => {
                    "partial OTEL import; the source telemetry captured message content for only some provider calls"
                }
                _ => "unrecognized OTEL capture level; message content capture is not established",
            };
            Some(format!(
                "--require-capture full: session capture level is '{}', not 'full' ({why})",
                sanitize_display(&info.capture_level)
            ))
        }
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

/// Formats an event timestamp as RFC 3339 UTC (whole seconds, as stored on the wire).
fn rfc3339(ts: time::OffsetDateTime) -> String {
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| ts.unix_timestamp().to_string())
}

/// Compares the manifest's (unsigned) session time window with the first/last event timestamps at
/// second precision. Disagreement is a WARNING, never a failure: the manifest fields are not
/// signed, and the authoritative times are the event timestamps inside the hash chain.
#[must_use]
pub fn metadata_warnings(contents: &BundleContents) -> Vec<ReportViolation> {
    let (Some(first), Some(last)) = (contents.events.first(), contents.events.last()) else {
        return Vec::new();
    };
    let session = &contents.manifest.session;
    let checks = [
        (
            CREATED_AT_MISMATCH_CATEGORY,
            "created_at",
            &session.created_at,
            "first",
            first.emitted_at,
        ),
        (
            ENDED_AT_MISMATCH_CATEGORY,
            "ended_at",
            &session.ended_at,
            "last",
            last.emitted_at,
        ),
    ];
    let mut warnings = Vec::new();
    for (category, field, declared, which, event_ts) in checks {
        let parsed =
            time::OffsetDateTime::parse(declared, &time::format_description::well_known::Rfc3339);
        let agrees = parsed.is_ok_and(|ts| ts.unix_timestamp() == event_ts.unix_timestamp());
        if !agrees {
            warnings.push(ReportViolation {
                category: category.to_owned(),
                event_hash: None,
                object_hash: None,
                message: format!(
                    "unsigned manifest session.{field} ({}) disagrees with the {which} event timestamp ({}); the event timestamp is the one covered by integrity",
                    sanitize_display(declared),
                    rfc3339(event_ts)
                ),
            });
        }
    }
    warnings
}

/// Assembles the final [`BundleVerifyReportV1`] from already-computed pieces.
///
/// Takes `passed`/`violations`/`signatures`/`operators` as inputs (rather than computing them)
/// so callers that skip straight to a failure report (for example an import-time integrity
/// failure, with no signature or operator checks run) can still produce a well-formed report.
/// The derived flags (`integrity_ok`, `signature_verified`, `operator_key_verified`) are computed
/// from those inputs so they can never disagree with the detailed lists.
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
    let manifest = &contents.manifest;
    let integrity_ok = violations
        .iter()
        .all(|v| v.category == CAPTURE_REQUIREMENT_CATEGORY);
    // A signature or attestation over a head the contents do not hash to authenticates nothing:
    // the top-level flags are gated on integrity so a JSON consumer reading only them cannot
    // mistake a tampered bundle for an authenticated one. The per-entry outcomes in
    // `signatures`/`operators` still show what each signature check found.
    let signature_verified = integrity_ok && signatures.iter().any(|s| s.outcome == "verified");
    let operator_key_verified = integrity_ok && operators.iter().any(|o| o.operator_key_verified);
    let session_window = match (contents.events.first(), contents.events.last()) {
        (Some(first), Some(last)) => Some(SessionWindow {
            first_event_at: rfc3339(first.emitted_at),
            last_event_at: rfc3339(last.emitted_at),
        }),
        _ => None,
    };
    BundleVerifyReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: manifest.agef_version.clone(),
        bundle_path: bundle_path_display,
        session_id: manifest.session.id.clone(),
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
        integrity_ok,
        signature_verified,
        operator_key_verified,
        hash_algorithm: manifest.hash_algorithm.clone(),
        head: manifest.session.head.clone(),
        session_window,
        unsigned_metadata: UnsignedMetadata {
            created_at: manifest.session.created_at.clone(),
            ended_at: manifest.session.ended_at.clone(),
            producer_name: manifest.producer.name.clone(),
            producer_version: manifest.producer.version.clone(),
        },
        warnings: metadata_warnings(contents),
    }
}

/// Trusted keys and `--require-*` requirements for one verification run, shared by both binaries.
#[derive(Debug, Clone, Default)]
pub struct VerifyPolicy {
    /// Trusted signer public keys (raw 32-byte Ed25519) from `--verify-key`.
    pub trusted_keys: Vec<Vec<u8>>,
    /// `--require-signature`.
    pub require_signature: bool,
    /// Trusted operator keys: the union of `--operator-key` and `--require-operator-key`.
    pub operator_trusted_keys: Vec<Vec<u8>>,
    /// `--require-operator`.
    pub require_operator: bool,
    /// `key_id`s from `--require-operator-key` that MUST have a verified attestation.
    pub required_operator_key_ids: Vec<String>,
    /// `--require-capture full`.
    pub require_capture_full: bool,
}

/// A key file that could not be loaded while building a [`VerifyPolicy`].
#[derive(Debug, Clone)]
pub struct PolicyKeyError {
    /// Stable category: `verify_key_error` or `operator_key_error`.
    pub category: &'static str,
    /// Human-readable description.
    pub message: String,
}

impl VerifyPolicy {
    /// Loads every key file named on the command line and records the requirements.
    ///
    /// # Errors
    ///
    /// Returns the first key file that cannot be read or parsed, with its stable category.
    pub fn load(
        verify_keys: &[PathBuf],
        require_signature: bool,
        operator_keys: &[PathBuf],
        require_operator: bool,
        require_operator_keys: &[PathBuf],
        require_capture_full: bool,
    ) -> Result<Self, PolicyKeyError> {
        let verify_err = |message| PolicyKeyError {
            category: "verify_key_error",
            message,
        };
        let operator_err = |message| PolicyKeyError {
            category: "operator_key_error",
            message,
        };
        let mut policy = Self {
            require_signature,
            require_operator,
            require_capture_full,
            ..Self::default()
        };
        for path in verify_keys {
            policy
                .trusted_keys
                .push(load_verify_key(path).map_err(verify_err)?);
        }
        for path in operator_keys {
            policy
                .operator_trusted_keys
                .push(load_operator_key(path).map_err(operator_err)?);
        }
        // Each --require-operator-key is ALSO trusted for verification, and its key_id is recorded
        // so we can demand that THAT specific key produced a verified attestation.
        for path in require_operator_keys {
            let key = load_operator_key(path).map_err(operator_err)?;
            policy.required_operator_key_ids.push(key_id(&key));
            policy.operator_trusted_keys.push(key);
        }
        Ok(policy)
    }
}

/// Runs integrity, signature, operator-attestation, and capture checks on a parsed bundle and
/// returns the complete report. This is the whole verification decision, shared by both binaries.
#[must_use]
pub fn evaluate_bundle(
    bundle_path_display: String,
    contents: &BundleContents,
    policy: &VerifyPolicy,
) -> BundleVerifyReportV1 {
    let verification = verify_bundle(contents);
    let sig_report = verify_manifest_signatures(&contents.manifest, &policy.trusted_keys);
    let op_report = verify_operator_attestations(&contents.manifest, &policy.operator_trusted_keys);
    let operator_ok = operator_requirements_ok(
        &op_report,
        policy.require_operator,
        &policy.required_operator_key_ids,
    );
    let capture = otel_capture_info(contents);
    let (passed, violations) = compute_passed_and_violations(
        &verification,
        &sig_report,
        policy.require_signature,
        operator_ok,
        capture.as_ref(),
        policy.require_capture_full,
    );
    build_verify_report(
        bundle_path_display,
        contents,
        passed,
        violations,
        signature_reports(&sig_report),
        operator_reports(&op_report),
        capture.as_ref(),
    )
}
