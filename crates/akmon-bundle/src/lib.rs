//! AGEF bundle primitives for Akmon.
//!
//! This crate provides:
//! - `manifest.json` parsing and canonical JSON serialization.
//! - `events.bin` length-delimited canonical-CBOR framing.
//! - `objects/<hex>` path helpers and basic file I/O.
//! - `tar.zst` bundle read/write helpers.
//! - store-independent bundle integrity verification (see [`verify`]).
//! - optional Ed25519 detached session signatures (see [`signing`]; AGEF v0.1.2).

pub mod archive;
pub mod error;
pub mod events;
pub mod manifest;
pub mod objects;
pub mod sentinel;
pub mod signing;
pub mod verify;

pub use archive::{
    BundleContents, ReadBundleOptions, WriteBundleOptions, read_bundle, read_verified_bundle,
    write_bundle,
};
pub use error::BundleError;
pub use events::{DEFAULT_MAX_EVENT_FRAME_LEN, EventsReader, EventsWriter};
pub use manifest::{Manifest, ManifestSignature, Producer, SessionMetadata};
pub use objects::{object_filename, object_path, read_object_file, write_object_file};
pub use sentinel::{
    SentinelMarker, SentinelParseError, is_sentinel, sentinel_from_original,
    sentinel_to_canonical_cbor, try_parse_sentinel,
};
pub use signing::{
    SCHEME_ED25519, SIG_STATEMENT_VERSION, SignatureCheck, SignatureOutcome,
    SignatureVerificationReport, SigningError, generate_pkcs8, key_id, parse_public_key_hex,
    public_key_from_pkcs8, sign_statement, signing_statement, verify_manifest_signatures,
    verify_statement,
};
pub use verify::{BundleVerificationReport, BundleViolation, verify_bundle, verify_bundle_strict};
