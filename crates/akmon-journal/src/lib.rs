//! Content-addressed journal substrate for Akmon and AGEF v0.1.
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

pub mod error;
pub mod event;
pub mod hash;
mod journal_meta;
pub mod object_store;
pub mod prelude;
pub mod session_graph;

pub use error::{JournalError, Result};
pub use event::{AttemptRecord, AttemptStatus, Event, EventKind};
pub use hash::{BLAKE3_LEN, Hash, HashAlgorithm, SHA256_LEN, WireHash, digest_bytes};
pub use object_store::{MemoryObjectStore, ObjectStore, RedbObjectStore};
pub use session_graph::{RedbSessionGraph, SessionGraph, VerificationReport};
