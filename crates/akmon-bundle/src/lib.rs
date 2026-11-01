//! AGEF bundle primitives for Akmon.
//!
//! This crate provides:
//! - `manifest.json` parsing and canonical JSON serialization.
//! - `events.bin` length-delimited canonical-CBOR framing.
//! - `objects/<hex>` naming helper.
//! - `tar.zst` bundle read/write helpers.
//! - store-independent bundle integrity verification (see [`verify`]).
//! - optional Ed25519 detached session signatures (see [`signing`]; AGEF v0.1.2).
//! - the shared verification report and policy (see [`report`]) and its human/Markdown
//!   renderings (see [`render`]), with terminal-safe escaping of manifest strings (see [`sanitize`]).

pub mod archive;
pub mod capture;
pub mod error;
pub mod events;
pub mod manifest;
pub mod objects;
pub mod render;
pub mod report;
pub mod sanitize;
pub mod sentinel;
pub mod signing;
pub mod spki;
pub mod verify;

pub use archive::{
    BundleContents, DEFAULT_MAX_BUNDLE_DECODED_BYTES, ReadBundleOptions, WriteBundleOptions,
    parse_algorithm, read_bundle, read_bundle_limited, read_verified_bundle, write_bundle,
};
pub use capture::{OtelCaptureInfo, otel_capture_info};
pub use error::BundleError;
pub use events::{DEFAULT_MAX_EVENT_FRAME_LEN, EventsReader, EventsWriter};
pub use manifest::{Manifest, ManifestSignature, OperatorAttestation, Producer, SessionMetadata};
pub use objects::object_filename;
pub use render::{
    capture_human_line, capture_human_suffix, headline, human_lines, markdown, markdown_escape,
    print_operator_human_block,
};
pub use report::{
    BundleVerifyReportV1, CAPTURE_REQUIREMENT_CATEGORY, CREATED_AT_MISMATCH_CATEGORY, CaptureField,
    ENDED_AT_MISMATCH_CATEGORY, OperatorReport, PolicyKeyError, ReportViolation, SessionWindow,
    SignatureReport, ToolErrorV1, UnsignedMetadata, VerifyPolicy, build_verify_report,
    bundle_read_error_category, bundle_read_exit_code, capture_requirement_unmet,
    compute_passed_and_violations, evaluate_bundle, load_operator_key, load_verify_key,
    metadata_warnings, operator_outcome_str, operator_reports, operator_requirements_ok,
    report_violations, sanitize_operator_field, signature_outcome_str, signature_reports,
};
pub use sanitize::sanitize_display;
pub use sentinel::{
    SentinelMarker, SentinelParseError, is_sentinel, sentinel_from_original,
    sentinel_to_canonical_cbor, try_parse_sentinel,
};
pub use signing::{
    OPERATOR_STATEMENT_VERSION, OperatorCheck, OperatorIdentity, OperatorOutcome,
    OperatorVerificationReport, SCHEME_ED25519, SIG_STATEMENT_VERSION, SignatureCheck,
    SignatureOutcome, SignatureVerificationReport, SigningError, build_operator_attestation,
    generate_pkcs8, key_id, operator_statement, parse_public_key_hex, public_key_from_pkcs8,
    sign_statement, signing_statement, validate_operator_field, verify_manifest_signatures,
    verify_operator_attestations, verify_statement,
};
pub use spki::{ED25519_SPKI_DER_LEN, ed25519_spki_der, ed25519_spki_pem};
pub use verify::{BundleVerificationReport, BundleViolation, verify_bundle, verify_bundle_strict};
