//! Import OpenTelemetry GenAI traces into AGEF sessions (Akmon Item 9.1).
//!
//! Supported input forms (see [`SEMCONV_VERSION`]):
//!
//! - the structured GenAI form (v1.37 through the v1.42.x
//!   `semantic-conventions-genai` repository): `gen_ai.input.messages` /
//!   `gen_ai.output.messages` / `gen_ai.system_instructions` as span attributes
//!   (JSON strings or structured `arrayValue`/`kvlistValue`), or carried on a
//!   `gen_ai.client.inference.operation.details` span event;
//! - the legacy (<= v1.36) message-event form (`gen_ai.system.message` /
//!   `gen_ai.user.message` / `gen_ai.assistant.message` / `gen_ai.tool.message` /
//!   `gen_ai.choice`), reduced to the *same* canonical role+parts JSON, so the
//!   forms produce identical content-object hashes for identical content.
//!
//! Other current `gen_ai.*` events (for example `gen_ai.evaluation.result`) are
//! recorded as structural metadata in the signed config object; only
//! legacy-shaped message events that cannot be reduced are refused.
//!
//! An import has two phases. [`plan_otel_import`] parses and fully validates the
//! trace and pre-computes every event and object, with no side effects;
//! [`ImportPlan::emit`] then writes them to an [`ObjectStore`] and an empty
//! [`SessionGraph`]. Opening the session only after planning means a refused
//! trace never leaves an orphan session in the journal. The produced session is
//! a valid AGEF merkle chain that passes `graph.verify()?.is_clean()`.
//!
//! The capture level ([`CaptureLevel`]) is computed from the content slots that
//! were actually emitted and is baked into the session config object, and
//! therefore into the signed head.

#![warn(missing_docs)]

mod canonical;
mod content;
mod error;
mod objects;
mod otlp;
mod plan;
mod semconv;

pub use error::OtelImportError;
pub use plan::{DEFAULT_MAX_TRACE_BYTES, ImportOptions, ImportPlan};
pub use semconv::{CaptureLevel, SEMCONV_VERSION};

use akmon_journal::{
    AttemptRecord, EventKind, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
};
use plan::PlannedEvent;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

/// Outcome of importing one OTLP/JSON GenAI trace into an AGEF session.
#[derive(Debug, Clone)]
pub struct ImportReport {
    /// Identifier of the session the events were appended to.
    pub session_id: Uuid,
    /// Capture level recorded in the session (see [`CaptureLevel`]).
    pub capture_level: CaptureLevel,
    /// Number of provider-call events emitted.
    pub provider_calls: u64,
    /// Number of tool-call events emitted.
    pub tool_calls: u64,
    /// Number of provider/tool calls recorded as failed (span status ERROR,
    /// `error.type`, or an exception event).
    pub failed_calls: u64,
    /// Number of user/assistant turn events emitted (with real content).
    pub turns_emitted: u64,
    /// Number of provider calls for which no assistant turn was emitted because
    /// only metadata (no real output content) was available.
    pub turns_suppressed_no_content: u64,
    /// User messages present in an input history that were never emitted as a
    /// `UserTurn` (history that does not extend the previous call's history).
    pub user_messages_omitted: u64,
    /// Non-text assistant parts (reasoning, uri, blob, file, ...) kept only in
    /// the provider-call response object, not in the `AssistantTurn`.
    pub non_text_parts_omitted: u64,
    /// Byte-identical duplicate spans dropped (OTLP retry re-exports).
    pub duplicate_spans_dropped: u64,
    /// Spans that produced no event (non-GenAI spans, agent/workflow spans).
    pub spans_skipped: u64,
    /// Non-legacy `gen_ai.*` events recorded as structural metadata.
    pub structural_events_recorded: u64,
    /// Content slots filled with real captured content.
    pub content_slots_real: u64,
    /// Total content slots (request+response per provider call, input+output
    /// per tool call).
    pub content_slots_total: u64,
    /// The semconv version this importer supports ([`SEMCONV_VERSION`]).
    pub semconv_version: String,
    /// The producer-declared semconv version(s) from `schemaUrl`, or
    /// `"undeclared"`.
    pub source_semconv: String,
    /// Content forms found (`structured_attributes`, `inference_details_event`,
    /// `legacy_events`, `+`-joined), or `"none"`.
    pub source_form: String,
    /// The imported trace id, when the spans carried one.
    pub trace_id: Option<String>,
}

/// Parses and fully validates an OTLP/JSON GenAI trace without side effects.
///
/// # Errors
///
/// Any [`OtelImportError`] except `Io` and `Journal`: size cap, parse error,
/// unreducible legacy event, duplicate span ids, multiple traces without a
/// selector, unknown selected trace, multiple conversations, no recognised GenAI
/// spans, invalid timestamps, or an empty trace.
pub fn plan_otel_import(
    trace_json: &[u8],
    options: &ImportOptions,
) -> Result<ImportPlan, OtelImportError> {
    plan::build_plan(trace_json, options)
}

impl ImportPlan {
    /// Writes the planned objects and events into `store` and the fresh, empty
    /// `graph`, returning the import report.
    ///
    /// # Errors
    ///
    /// Returns [`OtelImportError::Journal`] when an object-store or graph
    /// operation fails.
    pub fn emit<S: ObjectStore, G: SessionGraph>(
        self,
        store: &S,
        graph: &mut G,
    ) -> Result<ImportReport, OtelImportError> {
        for event in self.events {
            let kind = match event {
                PlannedEvent::SessionStart { cwd, config } => EventKind::SessionStart {
                    cwd_hash: store.put(&cwd)?,
                    config_hash: store.put(&config)?,
                },
                PlannedEvent::UserTurn { prompt } => EventKind::UserTurn {
                    prompt_hash: store.put(&prompt)?,
                },
                PlannedEvent::ProviderCall {
                    provider_id,
                    started_at,
                    ended_at,
                    status,
                    error_message,
                    request,
                    response,
                } => EventKind::ProviderCall {
                    provider_id,
                    attempts: vec![AttemptRecord {
                        attempt_number: 1,
                        started_at,
                        ended_at,
                        status,
                        request_hash: store.put(&request)?,
                        response_hash: Some(store.put(&response)?),
                        stream_hash: None,
                        error_message,
                    }],
                    stream_hash: None,
                },
                PlannedEvent::AssistantTurn {
                    message,
                    tool_calls,
                } => EventKind::AssistantTurn {
                    message_hash: store.put(&message)?,
                    tool_calls_hash: match tool_calls {
                        Some(tc) => Some(store.put(&tc)?),
                        None => None,
                    },
                },
                PlannedEvent::ToolCall {
                    tool_id,
                    input,
                    output,
                } => EventKind::ToolCall {
                    tool_id,
                    input_hash: store.put(&input)?,
                    output_hash: store.put(&output)?,
                    side_effects_hash: None,
                },
                PlannedEvent::SessionEnd => EventKind::SessionEnd { summary_hash: None },
            };
            graph.append(kind)?;
        }
        let s = self.stats;
        Ok(ImportReport {
            session_id: graph.session_id(),
            capture_level: self.capture_level,
            provider_calls: s.provider_calls,
            tool_calls: s.tool_calls,
            failed_calls: s.failed_calls,
            turns_emitted: s.turns_emitted,
            turns_suppressed_no_content: s.turns_suppressed_no_content,
            user_messages_omitted: s.user_messages_omitted,
            non_text_parts_omitted: s.non_text_parts_omitted,
            duplicate_spans_dropped: s.duplicate_spans_dropped,
            spans_skipped: s.spans_skipped,
            structural_events_recorded: s.structural_events_recorded,
            content_slots_real: s.content_slots_real,
            content_slots_total: s.content_slots_total,
            semconv_version: SEMCONV_VERSION.to_owned(),
            source_semconv: self.source_semconv,
            source_form: self.source_form,
            trace_id: self.trace_id,
        })
    }
}

/// Imports an OTLP/JSON GenAI trace into a fresh, empty AGEF session with
/// default [`ImportOptions`].
///
/// Equivalent to [`plan_otel_import`] followed by [`ImportPlan::emit`]; nothing
/// is written unless planning succeeds.
///
/// # Errors
///
/// See [`plan_otel_import`] and [`ImportPlan::emit`].
pub fn import_otel_genai<S: ObjectStore, G: SessionGraph>(
    trace_json: &[u8],
    store: &S,
    graph: &mut G,
) -> Result<ImportReport, OtelImportError> {
    plan_otel_import(trace_json, &ImportOptions::default())?.emit(store, graph)
}

/// Reads a trace file, refusing inputs larger than `max_bytes` (OTEL-9).
///
/// The size is checked from file metadata before reading, and the read itself
/// is bounded, so a file that grows while being read cannot exceed the cap.
///
/// # Errors
///
/// [`OtelImportError::Io`] when the file cannot be read, or
/// [`OtelImportError::TooLarge`] when it exceeds `max_bytes`.
pub fn read_trace_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, OtelImportError> {
    let io = |err: std::io::Error| OtelImportError::Io(format!("{}: {err}", path.display()));
    let file = std::fs::File::open(path).map_err(io)?;
    let size = file.metadata().map_err(io)?.len();
    if size > max_bytes {
        return Err(OtelImportError::TooLarge {
            size,
            max: max_bytes,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io)?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > max_bytes {
        return Err(OtelImportError::TooLarge {
            size: read,
            max: max_bytes,
        });
    }
    Ok(bytes)
}

/// Imports a trace file into a new session of an open journal object store.
///
/// This is the one-call entry point for CLI commands (`akmon otel import`,
/// `akmon seal`): it reads the file under the `options.max_bytes` cap, plans and
/// validates the import, and only then opens a fresh session (new random UUID)
/// and writes it. The returned report carries the session id and capture level.
///
/// # Errors
///
/// Any [`OtelImportError`]. Input errors are detected before the session is
/// created, so they never leave an orphan session behind.
pub fn import_trace_file_into_store(
    trace_path: &Path,
    store: Arc<RedbObjectStore>,
    options: &ImportOptions,
) -> Result<ImportReport, OtelImportError> {
    let bytes = read_trace_file(trace_path, options.max_bytes)?;
    let plan = plan_otel_import(&bytes, options)?;
    drop(bytes);
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), Uuid::new_v4())?;
    plan.emit(store.as_ref(), &mut graph)
}

#[cfg(test)]
mod tests;
