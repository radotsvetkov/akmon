//! Errors for index I/O and (when enabled) embedding.

/// Failure building, embedding, or persisting an index.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// Underlying filesystem error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Embedding model or batch call failed.
    #[cfg(feature = "semantic-index")]
    #[error("Embedding error: {0}")]
    Embedding(String),
    /// `bincode` encode/decode failure.
    #[error("Serialization error: {0}")]
    Serialization(String),
}
