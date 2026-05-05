//! AGEF bundle primitives for Akmon.
//!
//! This crate provides:
//! - `manifest.json` parsing and canonical JSON serialization.
//! - `events.bin` length-delimited canonical-CBOR framing.
//! - `objects/<hex>` path helpers and basic file I/O.
//! - `tar.zst` bundle read/write helpers.

pub mod archive;
pub mod error;
pub mod events;
pub mod manifest;
pub mod objects;
pub mod sentinel;

pub use archive::{
    BundleContents, ReadBundleOptions, WriteBundleOptions, read_bundle, write_bundle,
};
pub use error::BundleError;
pub use events::{DEFAULT_MAX_EVENT_FRAME_LEN, EventsReader, EventsWriter};
pub use manifest::{Manifest, Producer, SessionMetadata};
pub use objects::{object_filename, object_path, read_object_file, write_object_file};
pub use sentinel::{
    SentinelMarker, SentinelParseError, is_sentinel, sentinel_from_original,
    sentinel_to_canonical_cbor, try_parse_sentinel,
};
