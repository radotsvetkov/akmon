//! Content-addressed journal substrate for Akmon and AGEF v0.1.1.
//!
//! `akmon-journal` is the reference storage substrate for Akmon's AGEF evidence model:
//! immutable content-addressed objects plus merkle-linked session events.
//!
//! Serialization boundary:
//! - Internal persistence uses `postcard` for compact redb storage.
//! - Event hashing / AGEF wire compatibility uses canonical CBOR bytes.
//! - TODO(Item 4.3): bundle import/export owns external AGEF wire validation semantics, including
//!   explicit unknown EventKind/AttemptStatus rejection at the bundle boundary.
//!
//! Quickstart: import common APIs from [`prelude`] and create a [`RedbObjectStore`], then
//! open a [`RedbSessionGraph`] and append [`EventKind`] values.
//!
//! Verification (v2.0): [`session_graph::SessionGraph::verify`] recomputes digests for stored
//! object bytes (AGEF Section 13 step 5) and enforces terminal [`EventKind::SessionEnd`] invariants.

pub mod error;
pub mod event;
pub mod hash;
mod journal_meta;
pub mod object_store;
pub mod prelude;
pub mod session_graph;

/// AGEF specification version this crate implements.
///
/// Update when bumping the spec version (see <https://github.com/radotsvetkov/agef>). The CLI's
/// `VerifyReportV1.agef_version` field reads from this constant.
pub const AGEF_SPEC_VERSION: &str = "0.1.1";

pub use error::{JournalError, Result};
pub use event::{
    AttemptRecord, AttemptStatus, Event, EventKind, referenced_object_hashes_for_kind,
};
pub use hash::{BLAKE3_LEN, Hash, HashAlgorithm, SHA256_LEN, WireHash, digest_bytes};
pub use journal_meta::JournalMeta;
pub use object_store::{MemoryObjectStore, ObjectStore, RedbObjectStore};
#[cfg(any(test, feature = "test-utils"))]
pub use session_graph::MemorySessionGraph;
pub use session_graph::{
    MissingObject, RedbSessionGraph, SessionGraph, VerificationReport, VerifyCheck,
    session_head_row_exists, verify_linear_history_against_store,
};
