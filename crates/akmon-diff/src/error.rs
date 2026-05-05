use thiserror::Error;

/// Setup-time diff construction and loading errors.
#[derive(Debug, Error)]
pub enum DiffError {
    /// A source session could not be loaded from journal storage.
    #[error("failed to load source session {session_id}: {source}")]
    SourceSessionLoadFailed {
        /// Session UUID.
        session_id: String,
        /// Error detail from source loading.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// A source session UUID does not exist in the selected journal.
    #[error("source session not found: {session_id}")]
    SourceSessionMissing {
        /// Session UUID.
        session_id: String,
    },
    /// Object-store or journal access failed.
    #[error("store access failed: {source}")]
    StoreAccessFailed {
        /// Error detail from store access.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// Input session identifier is not a valid UUID for diff.
    #[error("invalid session id `{session_id}`: {reason}")]
    InvalidSessionId {
        /// Raw provided session identifier.
        session_id: String,
        /// Parsing or validation failure reason.
        reason: String,
    },
    /// Source session violates diff preconditions.
    #[error("source {session_label} precondition violated: {violation}")]
    SourcePreconditionViolated {
        /// Source label in pairwise comparison ("A" or "B").
        session_label: String,
        /// Human-readable precondition failure detail.
        violation: String,
    },
    /// Internal invariant violation in diff orchestration.
    #[error("internal diff error: {context}: {source}")]
    InternalError {
        /// Additional context for diagnostics.
        context: String,
        /// Source error detail.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

#[cfg(test)]
mod tests {
    use super::DiffError;

    #[test]
    fn t_display_non_empty_for_all_variants() {
        let values = vec![
            DiffError::SourceSessionLoadFailed {
                session_id: "s-a".to_owned(),
                source: std::io::Error::other("io").into(),
            },
            DiffError::SourceSessionMissing {
                session_id: "s-b".to_owned(),
            },
            DiffError::StoreAccessFailed {
                source: std::io::Error::other("perm denied").into(),
            },
            DiffError::InvalidSessionId {
                session_id: "bad".to_owned(),
                reason: "not uuid".to_owned(),
            },
            DiffError::SourcePreconditionViolated {
                session_label: "A".to_owned(),
                violation: "history is empty".to_owned(),
            },
            DiffError::InternalError {
                context: "walker".to_owned(),
                source: std::io::Error::other("state").into(),
            },
        ];
        for value in values {
            assert!(!value.to_string().trim().is_empty());
        }
    }
}
