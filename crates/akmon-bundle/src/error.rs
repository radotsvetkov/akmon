//! Error types for AGEF bundle parsing and validation.

/// Errors emitted when reading, validating, or writing AGEF bundles.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// Underlying filesystem or stream I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Archive container is malformed or missing required entries.
    #[error("invalid archive: {0}")]
    InvalidArchive(String),

    /// zstd stream cannot be decoded/encoded correctly.
    #[error("invalid compression stream: {0}")]
    InvalidCompression(String),

    /// `manifest.json` is invalid or violates semantic requirements.
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    /// Bundle declares unsupported AGEF version.
    #[error("unsupported AGEF version: {0}")]
    UnsupportedAgefVersion(String),

    /// Bundle declares unsupported hash algorithm.
    #[error("unsupported hash algorithm: {0}")]
    UnsupportedHashAlgorithm(String),

    /// `events.bin` framing is malformed.
    #[error("malformed framing: {0}")]
    MalformedFraming(String),

    /// One event frame exceeds configured safety limit.
    #[error("event frame exceeds max length: {0}")]
    FrameTooLarge(u32),

    /// Event CBOR bytes are valid but not canonical.
    #[error("non-canonical CBOR event frame")]
    NonCanonicalCbor,

    /// Event CBOR payload uses unknown EventKind variant.
    #[error("unknown EventKind: {0}")]
    UnknownEventKind(String),

    /// Event CBOR payload uses unknown AttemptStatus variant.
    #[error("unknown AttemptStatus: {0}")]
    UnknownAttemptStatus(String),

    /// Bundle references an object that is not present.
    #[error("missing object: {0}")]
    MissingObject(String),

    /// Object bytes do not hash to expected digest.
    #[error("object hash mismatch: {0}")]
    ObjectHashMismatch(String),

    /// Manifest head does not match computed terminal event hash.
    #[error("head mismatch: expected {expected}, found {found}")]
    HeadMismatch { expected: String, found: String },

    /// Bundle contains unknown non-normative files while strict mode is active.
    #[error("unknown file in bundle: {0}")]
    UnknownBundleFile(String),
}
