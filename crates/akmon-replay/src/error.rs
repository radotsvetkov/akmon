use akmon_journal::Hash;
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
}
