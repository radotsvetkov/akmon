//! Error type for OpenTelemetry GenAI trace import.

/// Errors produced while importing an OTLP/JSON GenAI trace into an AGEF session.
#[derive(Debug, thiserror::Error)]
pub enum OtelImportError {
    /// The input bytes are not a well-formed OTLP/JSON `ExportTraceServiceRequest`.
    #[error("otel trace parse error: {0}")]
    Parse(String),
    /// The trace uses the legacy (semconv <= v1.36) message-event form, which is
    /// not supported; re-export with semconv >= 1.37 structured attributes.
    #[error(
        "legacy <=v1.36 message-event form detected; re-export with semconv >=1.37 structured GenAI attributes"
    )]
    LegacySemconvUnsupported,
    /// The trace contains more than one `gen_ai.conversation.id`, implying
    /// multiple sessions. v1 imports exactly one session per trace and refuses to
    /// silently merge distinct conversations.
    #[error("multiple gen_ai.conversation.id values found; v1 imports one session per trace")]
    MultipleSessions,
    /// An underlying journal/object-store/graph operation failed.
    #[error(transparent)]
    Journal(#[from] akmon_journal::JournalError),
    /// The trace contains no spans to import.
    #[error("empty trace: no spans found in resourceSpans")]
    EmptyTrace,
}
