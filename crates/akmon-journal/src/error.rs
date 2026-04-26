//! Error types for the journal substrate.

use crate::hash::{Hash, HashAlgorithm};

/// Result alias for journal operations.
pub type Result<T> = std::result::Result<T, JournalError>;

/// Errors produced by journal storage, hashing, and verification flows.
/// TODO(Item 4.x): split broad `Verification(String)` into typed variants for
/// user-facing CLI diagnostics (already-exists, uninitialized, schema-mismatch, corruption).
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// Input/output failure while reading or writing journal data.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// Generic redb storage failure.
    /// Boxed to keep `JournalError` small enough for `Result<T, JournalError>` clippy constraints.
    #[error("storage error: {0}")]
    Storage(#[from] Box<redb::Error>),
    /// Redb transaction failure.
    /// Boxed to keep `JournalError` small enough for `Result<T, JournalError>` clippy constraints.
    #[error("storage transaction error: {0}")]
    StorageTx(#[from] Box<redb::TransactionError>),
    /// Postcard serialization/deserialization failure.
    #[error("postcard serialization error: {0}")]
    Postcard(#[from] postcard::Error),
    /// CBOR encoding/decoding failure.
    #[error("cbor serialization error: {0}")]
    Cbor(String),
    /// Hash algorithm mismatch between expected and observed values.
    #[error("hash algorithm mismatch: expected {expected:?}, found {found:?}")]
    HashAlgorithmMismatch {
        /// Expected hash algorithm.
        expected: HashAlgorithm,
        /// Found hash algorithm.
        found: HashAlgorithm,
    },
    /// Unknown hash algorithm string.
    #[error("unknown hash algorithm: {0}")]
    UnknownHashAlgorithm(String),
    /// Hash parsing or formatting failure.
    #[error("hash parse error: {0}")]
    HashParse(String),
    /// Requested session id does not exist in the journal.
    #[error("session not found: {0}")]
    SessionNotFound(uuid::Uuid),
    /// Event references a parent hash not present in graph state.
    #[error("event parent missing: {0}")]
    MissingParent(Hash),
    /// Verification failed due to structural or hash integrity issues.
    #[error("verification failed: {0}")]
    Verification(String),
    /// Write contention or exclusive access conflict.
    #[error("concurrent writer busy")]
    Busy,
}
