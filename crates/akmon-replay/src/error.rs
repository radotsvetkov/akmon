use akmon_journal::Hash;
use std::path::PathBuf;
use thiserror::Error;

/// Setup-time replay construction errors.
///
/// This type is reserved for constructor/setup failures. Runtime mismatches are
/// represented as [`crate::ReplayDivergence`] values and collected by the divergence sink.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// Source events list is empty.
    #[error("source event history is empty")]
    EmptySource,
    /// Source events contain no calls for the requested provider/tool identifier.
    #[error("source history contains no matching calls for `{0}`")]
    NoMatchingCalls(String),
    /// Source references a provider id that cannot be constructed for replay.
    #[error("missing provider for replay: {provider_id}")]
    MissingProviderForReplay {
        /// Provider identifier from source `ProviderCall`.
        provider_id: String,
    },
    /// Source references a tool id that cannot be constructed for replay.
    #[error("missing tool for replay: {tool_id}")]
    MissingToolForReplay {
        /// Tool identifier from source `ToolCall`.
        tool_id: String,
    },
    /// Source events reference an object hash not present in the provided store.
    #[error("source object missing from store: {0}")]
    MissingSourceObject(Hash),
    /// A source event has malformed structure preventing replay setup.
    #[error("malformed source event at sequence {event_seq}: {reason}")]
    MalformedSourceEvent {
        /// Event sequence number.
        event_seq: u64,
        /// Malformation reason.
        reason: String,
    },
    /// Source `SessionStart.config_hash` points to malformed config bytes.
    #[error("malformed source config at {config_hash}: {reason}")]
    MalformedSourceConfig {
        /// Config object hash from `SessionStart`.
        config_hash: Hash,
        /// Decode/parsing failure reason.
        reason: String,
    },
    /// Object-store read failed during setup.
    #[error("object-store read failed for {hash}: {reason}")]
    StoreReadFailed {
        /// Hash requested from object store.
        hash: Hash,
        /// Error detail.
        reason: String,
    },
    /// Replay engine currently expects one provider for one replay run.
    #[error("replay provider multiplicity unsupported (count={count})")]
    UnsupportedProviderMultiplicity {
        /// Number of providers discovered in source session.
        count: usize,
    },
    /// Replay engine failed while driving AgentSession for one source user turn.
    #[error("replay session run failed: {reason}")]
    SessionRunFailed {
        /// Failure detail.
        reason: String,
    },
    /// Replay session history is malformed after orchestration.
    #[error("replay session malformed: {reason}")]
    ReplaySessionMalformed {
        /// Validation detail.
        reason: String,
    },
    /// Persist config is invalid for requested replay mode.
    #[error("invalid replay persist configuration: {reason}")]
    PersistConfigInvalid {
        /// Validation detail.
        reason: String,
    },
    /// Persist target journal cannot be prepared for writing replay output.
    #[error("persist journal not writable at {path}: {reason}")]
    PersistJournalNotWritable {
        /// Target journal directory.
        path: PathBuf,
        /// Error detail.
        reason: String,
    },
}
