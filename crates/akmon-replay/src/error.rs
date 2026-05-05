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
    /// Object-store read failed during setup.
    #[error("object-store read failed for {hash}: {reason}")]
    StoreReadFailed {
        /// Hash requested from object store.
        hash: Hash,
        /// Error detail.
        reason: String,
    },
}
