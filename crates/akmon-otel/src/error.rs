//! Error type for OpenTelemetry GenAI trace import.

/// Errors produced while importing an OTLP/JSON GenAI trace into an AGEF session.
///
/// Every variant except [`OtelImportError::Journal`] and [`OtelImportError::Io`]
/// is detected while *planning* the import, before anything is written to the
/// journal, so a refused trace never leaves a session behind (OTEL-8).
#[derive(Debug, thiserror::Error)]
pub enum OtelImportError {
    /// The input bytes are not a well-formed OTLP/JSON `ExportTraceServiceRequest`.
    #[error("otel trace parse error: {0}")]
    Parse(String),
    /// A legacy-shaped `gen_ai.<role>.message` span event was found that is not
    /// one of the five supported legacy message events and cannot be reduced to
    /// structured content. Current-convention `gen_ai.*` events are never refused.
    #[error(
        "unrecognized legacy gen_ai message event '{0}' cannot be reduced to structured content (supported legacy events: gen_ai.system.message, gen_ai.user.message, gen_ai.assistant.message, gen_ai.tool.message, gen_ai.choice)"
    )]
    LegacySemconvUnsupported(String),
    /// The trace contains more than one `gen_ai.conversation.id`, implying
    /// multiple sessions. One session is imported per trace; distinct
    /// conversations are never silently merged.
    #[error(
        "multiple gen_ai.conversation.id values found ({0}); one session is imported per trace"
    )]
    MultipleSessions(String),
    /// The input holds spans from more than one trace and no trace was selected.
    #[error(
        "input contains {count} distinct trace ids ({ids}); select one with --trace-id (library: ImportOptions::trace_id)"
    )]
    MultipleTraces {
        /// Number of distinct trace ids.
        count: usize,
        /// The trace ids, comma-separated.
        ids: String,
    },
    /// The selected trace id does not occur in the input.
    #[error("trace id '{requested}' not found in input (available: {available})")]
    TraceIdNotFound {
        /// The requested trace id.
        requested: String,
        /// The trace ids present, comma-separated.
        available: String,
    },
    /// Two spans share a `(trace_id, span_id)` but differ in content.
    #[error("duplicate span id {span_id} in trace {trace_id} with differing content")]
    DuplicateSpanId {
        /// Trace id of the duplicated span.
        trace_id: String,
        /// The duplicated span id.
        span_id: String,
    },
    /// No span maps to a provider call or tool execution, so the session would
    /// carry no evidence.
    #[error(
        "no recognised GenAI spans: none of the {spans} span(s) has a gen_ai.operation.name of chat, text_completion, generate_content, embeddings or execute_tool"
    )]
    NoGenAiSpans {
        /// Number of spans in the (selected) trace.
        spans: usize,
    },
    /// A GenAI span has a missing, malformed, negative, unrepresentable or
    /// inverted (end before start) timestamp. Timestamps are never fabricated.
    #[error("span {span_id}: invalid timestamp: {reason}")]
    InvalidTimestamp {
        /// The offending span id.
        span_id: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The trace input exceeds the configured size cap.
    #[error("trace input is {size} bytes, over the {max}-byte limit (raise it with --max-bytes)")]
    TooLarge {
        /// Input size in bytes.
        size: u64,
        /// The configured cap in bytes.
        max: u64,
    },
    /// The trace file could not be read.
    #[error("cannot read trace file: {0}")]
    Io(String),
    /// An underlying journal/object-store/graph operation failed.
    #[error(transparent)]
    Journal(#[from] akmon_journal::JournalError),
    /// The trace contains no spans to import.
    #[error("empty trace: no spans found in resourceSpans")]
    EmptyTrace,
}

impl OtelImportError {
    /// Stable machine-readable category for this error.
    #[must_use]
    pub fn category(&self) -> &'static str {
        match self {
            Self::Parse(_) => "parse_error",
            Self::LegacySemconvUnsupported(_) => "legacy_semconv_unsupported",
            Self::MultipleSessions(_) => "multiple_sessions",
            Self::MultipleTraces { .. } => "multiple_traces",
            Self::TraceIdNotFound { .. } => "trace_id_not_found",
            Self::DuplicateSpanId { .. } => "duplicate_span_id",
            Self::NoGenAiSpans { .. } => "no_genai_spans",
            Self::InvalidTimestamp { .. } => "invalid_timestamp",
            Self::TooLarge { .. } => "trace_too_large",
            Self::Io(_) => "io_error",
            Self::Journal(_) => "journal_error",
            Self::EmptyTrace => "empty_trace",
        }
    }

    /// True when the error is caused by the input (a usage error), false for
    /// I/O or journal failures.
    #[must_use]
    pub fn is_input_error(&self) -> bool {
        !matches!(self, Self::Io(_) | Self::Journal(_))
    }
}
