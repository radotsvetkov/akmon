//! Pure planning step: parse, validate, order and build every event and content
//! object of the import *before* anything touches the journal (OTEL-8).
//!
//! [`build_plan`] performs all validation (size cap, parse, legacy refusal,
//! duplicate spans, trace selection, conversation uniqueness, recognised GenAI
//! spans, timestamps) and pre-computes every object's bytes. The emit step in
//! `lib.rs` then only stores bytes and appends events, so the only failures that
//! can happen after a session is opened are journal I/O failures.

use crate::content::{self, ContentSources, SpanContent, assistant_contents, content_value};
use crate::error::OtelImportError;
use crate::objects::{
    ConfigFacts, config_object_bytes, metadata_envelope_bytes, not_captured_bytes,
    real_content_bytes, tool_error_bytes,
};
use crate::otlp::{IntLike, NormalizedSpan, json_double, parse_and_normalize};
use crate::semconv::{self, CaptureLevel, GenAiEventClass, Operation, classify_event};
use akmon_journal::AttemptStatus;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use time::OffsetDateTime;

/// Default cap on the size of a trace input: 256 MiB (OTEL-9).
pub const DEFAULT_MAX_TRACE_BYTES: u64 = 256 * 1024 * 1024;

/// Options controlling an import.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Import only the spans of this trace id. Required when the input holds
    /// spans from more than one trace (OTEL-6). Compared case-insensitively.
    pub trace_id: Option<String>,
    /// Maximum accepted input size in bytes (default [`DEFAULT_MAX_TRACE_BYTES`]).
    pub max_bytes: u64,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            trace_id: None,
            max_bytes: DEFAULT_MAX_TRACE_BYTES,
        }
    }
}

/// One event of the plan, with its object bytes pre-computed.
#[derive(Debug)]
pub(crate) enum PlannedEvent {
    SessionStart {
        cwd: Vec<u8>,
        config: Vec<u8>,
    },
    UserTurn {
        prompt: Vec<u8>,
    },
    ProviderCall {
        provider_id: String,
        started_at: OffsetDateTime,
        ended_at: OffsetDateTime,
        status: AttemptStatus,
        error_message: Option<String>,
        request: Vec<u8>,
        response: Vec<u8>,
    },
    AssistantTurn {
        message: Vec<u8>,
        tool_calls: Option<Vec<u8>>,
    },
    ToolCall {
        tool_id: String,
        input: Vec<u8>,
        output: Vec<u8>,
    },
    SessionEnd,
}

/// Counters describing what the plan contains.
#[derive(Debug, Clone, Default)]
pub(crate) struct PlanStats {
    pub(crate) provider_calls: u64,
    pub(crate) tool_calls: u64,
    pub(crate) failed_calls: u64,
    pub(crate) turns_emitted: u64,
    pub(crate) turns_suppressed_no_content: u64,
    pub(crate) user_messages_omitted: u64,
    pub(crate) non_text_parts_omitted: u64,
    pub(crate) duplicate_spans_dropped: u64,
    pub(crate) spans_skipped: u64,
    pub(crate) structural_events_recorded: u64,
    pub(crate) content_slots_real: u64,
    pub(crate) content_slots_total: u64,
}

/// A fully validated import, ready to be written to a journal.
#[derive(Debug)]
pub struct ImportPlan {
    pub(crate) events: Vec<PlannedEvent>,
    pub(crate) capture_level: CaptureLevel,
    pub(crate) stats: PlanStats,
    pub(crate) source_semconv: String,
    pub(crate) source_form: String,
    pub(crate) trace_id: Option<String>,
}

impl ImportPlan {
    /// The capture level the session will record.
    #[must_use]
    pub fn capture_level(&self) -> CaptureLevel {
        self.capture_level
    }

    /// Number of provider-call events the session will contain.
    #[must_use]
    pub fn provider_calls(&self) -> u64 {
        self.stats.provider_calls
    }

    /// Number of tool-call events the session will contain.
    #[must_use]
    pub fn tool_calls(&self) -> u64 {
        self.stats.tool_calls
    }
}

/// Builds a validated [`ImportPlan`] from OTLP/JSON bytes without side effects.
pub(crate) fn build_plan(
    trace_json: &[u8],
    options: &ImportOptions,
) -> Result<ImportPlan, OtelImportError> {
    let size = u64::try_from(trace_json.len()).unwrap_or(u64::MAX);
    if size > options.max_bytes {
        return Err(OtelImportError::TooLarge {
            size,
            max: options.max_bytes,
        });
    }
    let parsed = parse_and_normalize(trace_json)?;
    if parsed.spans.is_empty() {
        return Err(OtelImportError::EmptyTrace);
    }
    // Select the trace FIRST, so content in a trace the user did not ask for (an
    // unknown legacy event, a conflicting duplicate span) cannot refuse the import.
    let (spans, trace_id) = select_trace(parsed.spans, options.trace_id.as_deref())?;
    let (spans, duplicates) = dedupe_spans(spans)?;
    refuse_unknown_legacy(&spans)?;
    let conversation_id = resolve_conversation(&spans)?;

    let emitting_count = spans
        .iter()
        .filter(|s| span_operation(s).is_some_and(Operation::emits_event))
        .count();
    if emitting_count == 0 {
        return Err(OtelImportError::NoGenAiSpans { spans: spans.len() });
    }
    let mut emitting: Vec<(&NormalizedSpan, Operation, SpanTimes)> =
        Vec::with_capacity(emitting_count);
    for span in &spans {
        if let Some(op) = span_operation(span).filter(|op| op.emits_event()) {
            emitting.push((span, op, span_times(span)?));
        }
    }

    // Total deterministic order: start asc, then tree depth (a parent before a
    // child starting in the same nanosecond), then span id.
    let parents: HashMap<&str, &str> = spans
        .iter()
        .map(|s| (s.span_id.as_str(), s.parent_span_id.as_str()))
        .collect();
    emitting.sort_by_cached_key(|(span, _, times)| {
        (
            times.start_nanos,
            depth(span, &parents),
            span.span_id.clone(),
        )
    });

    let mut builder = Builder {
        stats: PlanStats {
            duplicate_spans_dropped: duplicates,
            spans_skipped: u64::try_from(spans.len().saturating_sub(emitting.len()))
                .unwrap_or(u64::MAX),
            ..PlanStats::default()
        },
        ..Builder::default()
    };
    for (span, op, times) in &emitting {
        if *op == Operation::ExecuteTool {
            builder.tool_span(span);
        } else {
            builder.provider_span(span, *op, times.start, times.end);
        }
    }

    let capture_level = CaptureLevel::from_slots(
        builder.stats.content_slots_real,
        builder.stats.content_slots_total,
    );
    let structural_events = structural_events(&spans);
    builder.stats.structural_events_recorded =
        u64::try_from(structural_events.len()).unwrap_or(u64::MAX);
    let facts = ConfigFacts {
        provider: first_attr(&spans, semconv::PROVIDER_NAME)
            .or_else(|| first_attr(&spans, semconv::SYSTEM_DEPRECATED))
            .or_else(|| Some("unknown".to_owned())),
        model: first_attr(&spans, semconv::REQUEST_MODEL)
            .or_else(|| first_attr(&spans, semconv::RESPONSE_MODEL)),
        conversation_id,
        agent: spans
            .iter()
            .find(|s| {
                s.parent_span_id.is_empty()
                    && span_operation(s).is_some_and(Operation::is_session_root)
            })
            .map(|s| s.span_id.clone()),
        agent_identity: agent_identity(&spans),
        trace_id: trace_id.clone(),
        source_semconv: source_semconv(&parsed.schema_urls),
        source_form: source_form(builder.sources),
        structural_events,
    };
    let mut events = Vec::with_capacity(builder.events.len() + 2);
    events.push(PlannedEvent::SessionStart {
        cwd: not_captured_bytes("cwd", "otel traces carry no working directory"),
        config: config_object_bytes(capture_level, &facts),
    });
    events.append(&mut builder.events);
    events.push(PlannedEvent::SessionEnd);

    Ok(ImportPlan {
        events,
        capture_level,
        stats: builder.stats,
        source_semconv: facts.source_semconv,
        source_form: facts.source_form,
        trace_id,
    })
}

/// Accumulates planned events across spans.
#[derive(Default)]
struct Builder {
    events: Vec<PlannedEvent>,
    stats: PlanStats,
    sources: ContentSources,
    /// The previous provider span's input history (for trailing-user detection).
    prev_input: Vec<Value>,
    /// Canonical bytes of every user message already emitted as a `UserTurn`.
    emitted_users: HashSet<Vec<u8>>,
    last_user: Option<Vec<u8>>,
}

impl Builder {
    /// Plans a provider call (plus its user and assistant turns).
    ///
    /// Embeddings spans emit a provider call but add NO content slots: the semantic
    /// conventions define no input/output message content for embeddings, so such
    /// slots could never be filled and would pin every RAG trace at `partial`.
    fn provider_span(
        &mut self,
        span: &NormalizedSpan,
        op: Operation,
        started_at: OffsetDateTime,
        ended_at: OffsetDateTime,
    ) {
        let is_embeddings = op == Operation::Embeddings;
        let content = content::span_content(span);
        merge_sources(&mut self.sources, content.sources);
        self.user_turns(&content);

        let request = content.request();
        if !is_embeddings {
            // System instructions alone do not make the request slot real: the
            // user/input side of the conversation must have been captured.
            self.count_slot(content.input_messages.is_some());
            self.count_slot(content.output_messages.is_some());
        }
        let request = request.map_or_else(|| request_envelope(span), |v| real_content_bytes(&v));
        let response = content
            .output_messages
            .as_ref()
            .map_or_else(|| response_envelope(span), real_content_bytes);

        let (status, error_message) = match failure(span) {
            None => (AttemptStatus::Success, None),
            Some(f) => {
                self.stats.failed_calls += 1;
                (AttemptStatus::Other(f.label), f.message)
            }
        };
        let provider_id = span
            .attr_str(semconv::PROVIDER_NAME)
            .or_else(|| span.attr_str(semconv::SYSTEM_DEPRECATED))
            .unwrap_or("unknown")
            .to_owned();
        self.events.push(PlannedEvent::ProviderCall {
            provider_id,
            started_at,
            ended_at,
            status,
            error_message,
            request,
            response,
        });
        self.stats.provider_calls += 1;

        if is_embeddings {
            return;
        }
        let (turns, empty): (Vec<_>, Vec<_>) = assistant_contents(content.output_messages.as_ref())
            .into_iter()
            .partition(content::AssistantContent::has_real_content);
        // A choice with no text and no tool calls (only reasoning/uri/blob/file
        // parts, or nothing) is not emitted as an AssistantTurn; its parts stay in
        // the provider call's response object.
        for turn in &empty {
            self.stats.non_text_parts_omitted += turn.parts_not_in_turn;
        }
        if turns.is_empty() {
            self.stats.turns_suppressed_no_content += 1;
        }
        for turn in turns {
            self.stats.non_text_parts_omitted += turn.parts_not_in_turn;
            self.events.push(PlannedEvent::AssistantTurn {
                message: real_content_bytes(&turn.message),
                tool_calls: turn.tool_calls.as_ref().map(real_content_bytes),
            });
            self.stats.turns_emitted += 1;
        }
    }

    /// Plans `UserTurn`s for user messages that are new in this span (OTEL-12).
    ///
    /// When the previous provider span's input history is a prefix of this one
    /// (the usual multi-turn shape), every user message in the new suffix is
    /// emitted. Otherwise only the trailing user message is emitted (unless it
    /// repeats the last one emitted), and earlier user messages never emitted
    /// before are counted in `user_messages_omitted`.
    fn user_turns(&mut self, content: &SpanContent) {
        let input = content.input_slice();
        if input.is_empty() {
            return;
        }
        let new_users: Vec<&Value> = if input.starts_with(&self.prev_input) {
            input[self.prev_input.len()..]
                .iter()
                .filter(|m| content::is_user(m))
                .collect()
        } else {
            let users: Vec<&Value> = input.iter().filter(|m| content::is_user(m)).collect();
            let Some((last, earlier)) = users.split_last() else {
                self.prev_input = input.to_vec();
                return;
            };
            let omitted = earlier
                .iter()
                .filter(|m| !self.emitted_users.contains(&real_content_bytes(m)))
                .count();
            self.stats.user_messages_omitted += u64::try_from(omitted).unwrap_or(u64::MAX);
            let last_bytes = real_content_bytes(last);
            if self.last_user.as_ref() == Some(&last_bytes) {
                Vec::new()
            } else {
                vec![*last]
            }
        };
        for user in new_users {
            let prompt = real_content_bytes(user);
            self.emitted_users.insert(prompt.clone());
            self.last_user = Some(prompt.clone());
            self.events.push(PlannedEvent::UserTurn { prompt });
            self.stats.turns_emitted += 1;
        }
        self.prev_input = input.to_vec();
    }

    /// Plans a tool call for an `execute_tool` span.
    fn tool_span(&mut self, span: &NormalizedSpan) {
        let tool_name = span.attr_str(semconv::TOOL_NAME).unwrap_or("unknown");
        let args = content_value(span.attributes.get(semconv::TOOL_CALL_ARGUMENTS));
        let result = content_value(span.attributes.get(semconv::TOOL_CALL_RESULT));
        self.count_slot(args.is_some());
        self.count_slot(result.is_some());
        if args.is_some() || result.is_some() {
            self.sources.attributes = true;
        }

        let input = args.as_ref().map_or_else(
            || {
                let mut attrs = Map::new();
                attrs.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
                if let Some(call_id) = span.attr_str(semconv::TOOL_CALL_ID) {
                    attrs.insert("tool_call_id".to_owned(), Value::String(call_id.to_owned()));
                }
                metadata_envelope_bytes("tool_input", attrs)
            },
            real_content_bytes,
        );
        let output = if let Some(f) = failure(span) {
            self.stats.failed_calls += 1;
            let mut error = Map::new();
            error.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
            error.insert("error_type".to_owned(), Value::String(f.label));
            error.insert(
                "message".to_owned(),
                f.message.map_or(Value::Null, Value::String),
            );
            error.insert("status_error".to_owned(), Value::Bool(span.status.is_error));
            tool_error_bytes(error, result)
        } else {
            result.as_ref().map_or_else(
                || {
                    let mut attrs = Map::new();
                    attrs.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
                    metadata_envelope_bytes("tool_output", attrs)
                },
                real_content_bytes,
            )
        };
        self.events.push(PlannedEvent::ToolCall {
            tool_id: tool_name.to_owned(),
            input,
            output,
        });
        self.stats.tool_calls += 1;
    }

    fn count_slot(&mut self, real: bool) {
        self.stats.content_slots_total += 1;
        if real {
            self.stats.content_slots_real += 1;
        }
    }
}

fn merge_sources(into: &mut ContentSources, from: ContentSources) {
    into.attributes |= from.attributes;
    into.details_event |= from.details_event;
    into.legacy_events |= from.legacy_events;
}

/// Returns the recognized GenAI operation for a span, if any.
fn span_operation(span: &NormalizedSpan) -> Option<Operation> {
    span.attr_str(semconv::OPERATION_NAME)
        .and_then(Operation::from_name)
}

/// Refuses legacy-shaped message events we cannot reduce (OTEL-2 / F8).
fn refuse_unknown_legacy(spans: &[NormalizedSpan]) -> Result<(), OtelImportError> {
    let unknown = spans
        .iter()
        .flat_map(|s| s.events.iter())
        .find(|e| classify_event(&e.name) == GenAiEventClass::UnknownLegacy);
    match unknown {
        Some(event) => Err(OtelImportError::LegacySemconvUnsupported(
            event.name.clone(),
        )),
        None => Ok(()),
    }
}

/// Drops duplicate spans (OTLP retries re-export batches) and rejects differing
/// spans sharing a `(trace_id, span_id)` (OTEL-6).
///
/// "Duplicate" means *normalized-equal*, not byte-identical: two spans compare
/// equal when every field the importer parses (ids, parent, times, status,
/// attributes, events) is equal. Fields the importer does not parse (for example
/// dropped-attribute counts) are ignored, so spans differing only there are
/// treated as duplicates. Because attribute values compare with `f64` equality, a
/// retried span carrying a `doubleValue` of NaN never equals itself and is
/// rejected as [`OtelImportError::DuplicateSpanId`] rather than dropped.
fn dedupe_spans(spans: Vec<NormalizedSpan>) -> Result<(Vec<NormalizedSpan>, u64), OtelImportError> {
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut out: Vec<NormalizedSpan> = Vec::with_capacity(spans.len());
    let mut dropped = 0_u64;
    for span in spans {
        if span.span_id.is_empty() {
            out.push(span);
            continue;
        }
        let key = (
            span.trace_id.to_ascii_lowercase(),
            span.span_id.to_ascii_lowercase(),
        );
        if let Some(&idx) = seen.get(&key) {
            if out.get(idx) == Some(&span) {
                dropped += 1;
                continue;
            }
            return Err(OtelImportError::DuplicateSpanId {
                trace_id: span.trace_id,
                span_id: span.span_id,
            });
        }
        seen.insert(key, out.len());
        out.push(span);
    }
    Ok((out, dropped))
}

/// Restricts the spans to a single trace (OTEL-6).
fn select_trace(
    spans: Vec<NormalizedSpan>,
    requested: Option<&str>,
) -> Result<(Vec<NormalizedSpan>, Option<String>), OtelImportError> {
    let ids: BTreeSet<String> = spans
        .iter()
        .filter(|s| !s.trace_id.is_empty())
        .map(|s| s.trace_id.to_ascii_lowercase())
        .collect();
    let joined = || ids.iter().cloned().collect::<Vec<_>>().join(", ");
    match requested {
        Some(wanted) => {
            let wanted = wanted.trim().to_ascii_lowercase();
            if !ids.contains(&wanted) {
                return Err(OtelImportError::TraceIdNotFound {
                    requested: wanted,
                    available: joined(),
                });
            }
            let selected = spans
                .into_iter()
                .filter(|s| s.trace_id.to_ascii_lowercase() == wanted)
                .collect();
            Ok((selected, Some(wanted)))
        }
        None if ids.len() > 1 => Err(OtelImportError::MultipleTraces {
            count: ids.len(),
            ids: joined(),
        }),
        None => {
            let id = ids.into_iter().next();
            Ok((spans, id))
        }
    }
}

/// Rejects traces with more than one `gen_ai.conversation.id` (F6).
fn resolve_conversation(spans: &[NormalizedSpan]) -> Result<Option<String>, OtelImportError> {
    let ids: BTreeSet<&str> = spans
        .iter()
        .filter_map(|s| s.attr_str(semconv::CONVERSATION_ID))
        .collect();
    if ids.len() > 1 {
        return Err(OtelImportError::MultipleSessions(
            ids.into_iter().collect::<Vec<_>>().join(", "),
        ));
    }
    Ok(ids.into_iter().next().map(str::to_owned))
}

/// A span's validated timestamps.
#[derive(Debug, Clone, Copy)]
struct SpanTimes {
    start: OffsetDateTime,
    end: OffsetDateTime,
    start_nanos: i128,
}

/// Validates a span's timestamps (OTEL-10). Never fabricates an instant.
fn span_times(span: &NormalizedSpan) -> Result<SpanTimes, OtelImportError> {
    let bad = |reason: String| OtelImportError::InvalidTimestamp {
        span_id: span.span_id.clone(),
        reason,
    };
    let check = |field: &str, value: IntLike| -> Result<(i128, OffsetDateTime), OtelImportError> {
        let nanos = match value {
            IntLike::Missing => return Err(bad(format!("{field} is missing"))),
            IntLike::Invalid => return Err(bad(format!("{field} is not an integer"))),
            IntLike::Valid(0) => return Err(bad(format!("{field} is 0 (unset)"))),
            IntLike::Valid(n) if n < 0 => return Err(bad(format!("{field} is negative ({n})"))),
            IntLike::Valid(n) => n,
        };
        let time = OffsetDateTime::from_unix_timestamp_nanos(nanos)
            .map_err(|_| bad(format!("{field} is out of range ({nanos})")))?;
        Ok((nanos, time))
    };
    let (start_nanos, start) = check("startTimeUnixNano", span.start)?;
    let (end_nanos, end) = check("endTimeUnixNano", span.end)?;
    if end_nanos < start_nanos {
        return Err(bad(format!(
            "endTimeUnixNano ({end_nanos}) is before startTimeUnixNano ({start_nanos})"
        )));
    }
    Ok(SpanTimes {
        start,
        end,
        start_nanos,
    })
}

/// Number of ancestors of a span within the trace (cycle-safe).
fn depth(span: &NormalizedSpan, parents: &HashMap<&str, &str>) -> usize {
    let mut depth = 0;
    let mut current = span.parent_span_id.as_str();
    while !current.is_empty() && depth <= parents.len() {
        depth += 1;
        current = parents.get(current).copied().unwrap_or("");
    }
    depth
}

/// A recorded failure of a span (OTEL-5).
struct Failure {
    label: String,
    message: Option<String>,
}

/// Detects a failed span: status ERROR, an `error.type` attribute, or an
/// exception event.
fn failure(span: &NormalizedSpan) -> Option<Failure> {
    let exception = span
        .events
        .iter()
        .find(|e| classify_event(&e.name) == GenAiEventClass::Exception);
    let error_type = span.attr_str(semconv::ERROR_TYPE);
    if !span.status.is_error && error_type.is_none() && exception.is_none() {
        return None;
    }
    let exc_type = exception.and_then(|e| e.attr_str(semconv::EXCEPTION_TYPE));
    let exc_message = exception.and_then(|e| e.attr_str(semconv::EXCEPTION_MESSAGE));
    let label = error_type
        .or(exc_type)
        .unwrap_or(if span.status.is_error {
            "span_status_error"
        } else {
            "exception"
        })
        .to_owned();
    let message = span
        .status
        .message
        .clone()
        .or_else(|| exc_message.map(str::to_owned))
        .or_else(|| error_type.map(str::to_owned));
    Some(Failure { label, message })
}

/// Builds the request metadata envelope (no real input content).
fn request_envelope(span: &NormalizedSpan) -> Vec<u8> {
    let mut attrs = Map::new();
    insert_str(&mut attrs, "model", span.attr_str(semconv::REQUEST_MODEL));
    if let Some(temp) = span.attr_f64(semconv::REQUEST_TEMPERATURE) {
        attrs.insert("temperature".to_owned(), json_double(temp));
    }
    insert_int(
        &mut attrs,
        "max_tokens",
        span.attr_i64(semconv::REQUEST_MAX_TOKENS),
    );
    insert_int(
        &mut attrs,
        "input_tokens",
        span.attr_i64(semconv::USAGE_INPUT_TOKENS),
    );
    insert_int(
        &mut attrs,
        "cache_read_input_tokens",
        span.attr_i64(semconv::USAGE_CACHE_READ_INPUT_TOKENS),
    );
    insert_int(
        &mut attrs,
        "cache_creation_input_tokens",
        span.attr_i64(semconv::USAGE_CACHE_CREATION_INPUT_TOKENS),
    );
    if let Some(crate::otlp::AnyValue::Bool(stream)) = span.attributes.get(semconv::REQUEST_STREAM)
    {
        attrs.insert("stream".to_owned(), Value::Bool(*stream));
    }
    if let Some(defs) = content_value(span.attributes.get(semconv::TOOL_DEFINITIONS)) {
        attrs.insert("tool_definitions".to_owned(), defs);
    }
    metadata_envelope_bytes("request", attrs)
}

/// Builds the response metadata envelope (no real output content).
fn response_envelope(span: &NormalizedSpan) -> Vec<u8> {
    let mut attrs = Map::new();
    insert_str(&mut attrs, "model", span.attr_str(semconv::RESPONSE_MODEL));
    insert_int(
        &mut attrs,
        "output_tokens",
        span.attr_i64(semconv::USAGE_OUTPUT_TOKENS),
    );
    insert_int(
        &mut attrs,
        "reasoning_output_tokens",
        span.attr_i64(semconv::USAGE_REASONING_OUTPUT_TOKENS),
    );
    if let Some(reasons) = span.attr_array(semconv::FINISH_REASONS) {
        let reasons: Vec<Value> = reasons
            .iter()
            .filter_map(|v| v.as_str().map(|s| Value::String(s.to_owned())))
            .collect();
        attrs.insert("finish_reasons".to_owned(), Value::Array(reasons));
    }
    insert_str(
        &mut attrs,
        "response_id",
        span.attr_str(semconv::RESPONSE_ID),
    );
    insert_str(
        &mut attrs,
        "output_type",
        span.attr_str(semconv::OUTPUT_TYPE),
    );
    metadata_envelope_bytes("response", attrs)
}

fn insert_str(attrs: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(v) = value {
        attrs.insert(key.to_owned(), Value::String(v.to_owned()));
    }
}

fn insert_int(attrs: &mut Map<String, Value>, key: &str, value: Option<i64>) {
    if let Some(v) = value {
        attrs.insert(key.to_owned(), Value::from(v));
    }
}

/// First string value of `key` across spans.
fn first_attr(spans: &[NormalizedSpan], key: &str) -> Option<String> {
    spans
        .iter()
        .find_map(|s| s.attr_str(key))
        .map(str::to_owned)
}

/// Agent name/id/version from the first span carrying them.
fn agent_identity(spans: &[NormalizedSpan]) -> Map<String, Value> {
    let mut identity = Map::new();
    for (key, name) in [
        (semconv::AGENT_NAME, "name"),
        (semconv::AGENT_ID, "id"),
        (semconv::AGENT_VERSION, "version"),
    ] {
        insert_str(
            &mut identity,
            name,
            spans.iter().find_map(|s| s.attr_str(key)),
        );
    }
    identity
}

/// Non-legacy `gen_ai.*` events (e.g. `gen_ai.evaluation.result`) recorded as
/// structural metadata, in span source order.
fn structural_events(spans: &[NormalizedSpan]) -> Vec<Value> {
    let mut out = Vec::new();
    for span in spans {
        for event in &span.events {
            if classify_event(&event.name) == GenAiEventClass::StructuralMetadata {
                let mut obj = Map::new();
                obj.insert("span_id".to_owned(), Value::String(span.span_id.clone()));
                obj.insert("name".to_owned(), Value::String(event.name.clone()));
                obj.insert(
                    "attributes".to_owned(),
                    Value::Object(event.attributes_json()),
                );
                out.push(Value::Object(obj));
            }
        }
    }
    out
}

/// The producer-declared semconv version(s) from `schemaUrl`s, or `"undeclared"`.
fn source_semconv(schema_urls: &BTreeSet<String>) -> String {
    let versions: BTreeSet<&str> = schema_urls
        .iter()
        .filter_map(|url| url.trim_end_matches('/').rsplit('/').next())
        .filter(|v| !v.is_empty())
        .collect();
    if versions.is_empty() {
        "undeclared".to_owned()
    } else {
        versions.into_iter().collect::<Vec<_>>().join(",")
    }
}

/// Which content forms supplied content (`+`-joined), or `"none"`.
fn source_form(sources: ContentSources) -> String {
    let mut forms = Vec::new();
    if sources.attributes {
        forms.push("structured_attributes");
    }
    if sources.details_event {
        forms.push("inference_details_event");
    }
    if sources.legacy_events {
        forms.push("legacy_events");
    }
    if forms.is_empty() {
        "none".to_owned()
    } else {
        forms.join("+")
    }
}
