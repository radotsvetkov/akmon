//! Errors returned by model backends and completion streams.

use thiserror::Error;

/// Failure while talking to a model or consuming its output stream.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelError {
    /// The backend could not be reached or returned an unexpected transport failure.
    #[error("backend unavailable: {message}")]
    BackendUnavailable {
        /// Human-readable explanation (no secrets).
        message: String,
    },
    /// No first token (or first chunk) arrived before [`crate::CompletionConfig::first_token_deadline_ms`].
    #[error("first token deadline exceeded")]
    FirstTokenTimeout,
    /// The server asked the client to back off.
    #[error("rate limited")]
    RateLimited {
        /// `Retry-After` hint in seconds when provided by the server.
        retry_after_secs: Option<u64>,
    },
    /// Credentials were rejected or missing for a protected endpoint.
    #[error("authentication failed")]
    AuthError,
    /// The prompt could not fit in the model's context window.
    #[error("context window exceeded")]
    ContextWindowExceeded,
    /// The stream ended early, the connection dropped mid-response, or a line could not be parsed.
    #[error("stream interrupted: {message}")]
    StreamInterrupted {
        /// Transport, framing, or parse detail (no secrets).
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_nonempty(e: ModelError) {
        assert!(!e.to_string().trim().is_empty());
    }

    #[test]
    fn backend_unavailable_display() {
        assert_nonempty(ModelError::BackendUnavailable {
            message: "refused".into(),
        });
    }

    #[test]
    fn first_token_timeout_display() {
        assert_nonempty(ModelError::FirstTokenTimeout);
    }

    #[test]
    fn rate_limited_display() {
        assert_nonempty(ModelError::RateLimited {
            retry_after_secs: Some(60),
        });
        assert_nonempty(ModelError::RateLimited {
            retry_after_secs: None,
        });
    }

    #[test]
    fn auth_error_display() {
        assert_nonempty(ModelError::AuthError);
    }

    #[test]
    fn context_window_exceeded_display() {
        assert_nonempty(ModelError::ContextWindowExceeded);
    }

    #[test]
    fn stream_interrupted_display() {
        assert_nonempty(ModelError::StreamInterrupted {
            message: "reset".into(),
        });
    }
}
