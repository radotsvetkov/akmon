//! `akmon otel` — import OpenTelemetry GenAI traces into AGEF sessions (Item 9.1).
//!
//! `akmon otel import` ingests an OTLP/JSON OpenTelemetry GenAI trace (structured
//! semconv v1.37 .. v1.42.x form, or the legacy <= v1.36 message events) into a
//! fresh AGEF session in the local journal, so the producer-agnostic result
//! composes with `akmon bundle export` / `sign` / `verify` and the standalone
//! `agef-verify` tool.
//!
//! The trace is read under a size cap and fully validated *before* the journal
//! is opened, so a refused trace never creates a journal or an orphan session.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use akmon_journal::{HashAlgorithm, RedbObjectStore, RedbSessionGraph};
use akmon_otel::{
    CaptureLevel, DEFAULT_MAX_TRACE_BYTES, ImportOptions, ImportReport, OtelImportError,
    plan_otel_import, read_trace_file,
};
use akmon_query::{default_journal_dir, journal_db_path};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

/// Output format for `akmon otel import` status messages.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OtelImportFormat {
    /// Human-readable status messages.
    #[default]
    Human,
    /// Machine-readable JSON status messages.
    Json,
}

/// Arguments for `akmon otel`.
#[derive(Args, Debug, Clone)]
pub struct OtelArgs {
    /// OTel command to execute.
    #[command(subcommand)]
    pub(crate) command: OtelCommands,
}

/// Nested `akmon otel` subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum OtelCommands {
    /// Import an OTLP/JSON OpenTelemetry GenAI trace into a new AGEF session.
    #[command(
        long_about = "Import an OTLP/JSON OpenTelemetry GenAI trace into a new AGEF session.\n\n\
Parses a single OTLP `ExportTraceServiceRequest` JSON document carrying GenAI spans in the \
structured semconv form (v1.37 through the v1.42.x semantic-conventions-genai repository: \
`gen_ai.input.messages` / `gen_ai.output.messages` / `gen_ai.system_instructions` as span \
attributes or on a `gen_ai.client.inference.operation.details` event) or the supported legacy \
(<= v1.36) message-event form (`gen_ai.system.message` / `gen_ai.user.message` / \
`gen_ai.assistant.message` / `gen_ai.tool.message` / `gen_ai.choice`, reduced to the same \
structured content). Other current gen_ai.* events (e.g. `gen_ai.evaluation.result`) are \
recorded as structural metadata. Spans are mapped onto AGEF events in a deterministic order and \
appended to a fresh session in the local journal. The trace is fully validated before anything \
is written, so a refused trace leaves no session behind.\n\n\
The produced session is a valid AGEF merkle chain:\n\
  akmon otel import trace.json --journal ./j\n\
  akmon bundle export <session-id> --journal ./j --output session.akmon\n\
  akmon bundle sign session.akmon --key signer.pk8\n\
  akmon bundle verify session.akmon --verify-key signer.pub.hex   # or: agef-verify\n\n\
Capture-level honesty: GenAI message/tool content is opt-in in the OpenTelemetry conventions. \
The capture level counts the content slots actually emitted (request and response of every \
provider call, input and output of every tool call): capture_level=full when every slot holds \
real captured content, partial when only some do, structural when none do (metadata only). It \
is baked into the signed session head.\n\n\
Input rules: one trace per import (use --trace-id when the file holds several); byte-identical \
duplicate spans are dropped, differing duplicates are refused; GenAI spans must carry valid \
start/end timestamps; a trace with no chat/text_completion/generate_content/embeddings/\
execute_tool span is refused; span status ERROR, error.type or an exception event records the \
call as failed.\n\n\
Output: in human mode the new session id is printed alone on stdout (details on stderr), so \
`sid=$(akmon otel import trace.json)` works.\n\n\
Examples:\n\
  akmon otel import trace.json\n\
  akmon otel import trace.json --journal ~/audit/journal\n\
  akmon otel import batch.json --trace-id 4bf92f3577b34da6a3ce929d0e0e4736\n\
  akmon otel import trace.json --format json\n\n\
Exit codes:\n\
  0 — trace imported successfully\n\
  2 — input refused (unparseable, too large, unrecognized legacy gen_ai message event, multiple \
traces/sessions, duplicate span ids, no GenAI spans, invalid timestamps, empty trace)\n\
  3 — I/O or environment error (trace file unreadable, journal store/graph failure)"
    )]
    Import(OtelImportArgs),
}

/// Arguments for `akmon otel import`.
#[derive(Args, Debug, Clone)]
pub struct OtelImportArgs {
    /// Path to the OTLP/JSON OpenTelemetry GenAI trace file to import.
    pub(crate) trace: PathBuf,
    /// Path to the journal directory.
    ///
    /// Defaults to per-user journal location (`$XDG_STATE_HOME/akmon/journal`).
    #[arg(long)]
    pub(crate) journal: Option<PathBuf>,
    /// Import only the spans of this trace id (required when the file holds
    /// spans from more than one trace).
    #[arg(long, value_name = "HEX")]
    pub(crate) trace_id: Option<String>,
    /// Refuse trace files larger than this many bytes (default 256 MiB).
    #[arg(long, value_name = "BYTES", default_value_t = DEFAULT_MAX_TRACE_BYTES)]
    pub(crate) max_bytes: u64,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: OtelImportFormat,
}

/// Stable JSON shape for `akmon otel import --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct OtelImportReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// Hyphenated session UUID assigned to the imported session.
    session_id: String,
    /// Capture level: `full`, `partial`, or `structural`.
    capture_level: String,
    /// Content slots filled with real captured content.
    content_slots_real: u64,
    /// Total content slots.
    content_slots_total: u64,
    /// Number of provider-call events emitted.
    provider_calls: u64,
    /// Number of tool-call events emitted.
    tool_calls: u64,
    /// Number of provider/tool calls recorded as failed.
    failed_calls: u64,
    /// Number of user/assistant turn events emitted (with real content).
    turns_emitted: u64,
    /// Number of provider calls with no assistant turn (no real output content).
    turns_suppressed_no_content: u64,
    /// User messages in an input history never emitted as a user turn.
    user_messages_omitted: u64,
    /// Non-text assistant parts kept only in the provider response object.
    non_text_parts_omitted: u64,
    /// Byte-identical duplicate spans dropped.
    duplicate_spans_dropped: u64,
    /// Spans that produced no event.
    spans_skipped: u64,
    /// Non-legacy gen_ai.* events recorded as structural metadata.
    structural_events_recorded: u64,
    /// The semconv version this importer supports.
    semconv_version: String,
    /// The producer-declared semconv version(s), or `undeclared`.
    source_semconv: String,
    /// Content forms found, or `none`.
    source_form: String,
    /// The imported trace id, when present.
    trace_id: Option<String>,
    /// Resolved journal directory the session was written into.
    journal_path: String,
    /// The exact command that exports this session as a bundle.
    next_command: String,
}

/// JSON shape emitted when `akmon otel import` cannot complete.
#[derive(Debug, Serialize, Deserialize)]
pub struct OtelImportInfraError {
    /// CLI crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
}

/// Why an import into the journal failed, with its CLI category and exit code.
#[derive(Debug)]
pub(crate) struct OtelImportFailure {
    /// Stable error category for automation.
    pub(crate) category: &'static str,
    /// Human-readable description.
    pub(crate) message: String,
    /// Process exit code (2 = input refused, 3 = I/O / environment).
    pub(crate) exit_code: u8,
}

impl OtelImportFailure {
    fn env(category: &'static str, message: String) -> Self {
        Self {
            category,
            message,
            exit_code: 3,
        }
    }
}

impl From<OtelImportError> for OtelImportFailure {
    fn from(err: OtelImportError) -> Self {
        Self {
            category: err.category(),
            message: err.to_string(),
            exit_code: if err.is_input_error() { 2 } else { 3 },
        }
    }
}

/// A successful import: the report and the resolved journal directory.
#[derive(Debug)]
pub(crate) struct OtelImportOutcome {
    /// The importer's report (session id, capture level, counters).
    pub(crate) report: ImportReport,
    /// The journal directory the session was written into.
    pub(crate) journal_dir: PathBuf,
}

/// Imports a trace file into a new session of the journal at `journal`
/// (default: the per-user journal).
///
/// Shared by `akmon otel import` and other commands (e.g. `akmon seal`). The
/// trace is read under `options.max_bytes` and validated before the journal
/// directory, store or session are created, so a refused trace writes nothing.
pub(crate) fn import_trace_into_journal(
    trace: &Path,
    journal: Option<PathBuf>,
    options: &ImportOptions,
) -> Result<OtelImportOutcome, OtelImportFailure> {
    let bytes = read_trace_file(trace, options.max_bytes)?;
    let plan = plan_otel_import(&bytes, options)?;
    drop(bytes);

    let journal_dir = match journal {
        Some(path) => path,
        None => default_journal_dir().map_err(|err| {
            OtelImportFailure::env(
                "journal_not_found",
                format!("cannot resolve default journal directory: {err}"),
            )
        })?,
    };
    std::fs::create_dir_all(journal_dir.as_path()).map_err(|err| {
        OtelImportFailure::env(
            "journal_access",
            format!(
                "cannot create journal directory {}: {err}",
                journal_dir.display()
            ),
        )
    })?;

    let journal_db = journal_db_path(journal_dir.as_path());
    let store = if journal_db.is_file() {
        RedbObjectStore::open(journal_db.as_path()).map_err(|err| {
            let msg = format!("cannot open journal store {}: {err}", journal_db.display());
            OtelImportFailure::env(
                crate::journal_errors::journal_access_category(&msg),
                crate::journal_errors::with_journal_lock_hint(msg),
            )
        })?
    } else {
        RedbObjectStore::create(journal_db.as_path(), HashAlgorithm::Sha256).map_err(|err| {
            OtelImportFailure::env(
                "journal_access",
                format!(
                    "cannot create journal store {}: {err}",
                    journal_db.display()
                ),
            )
        })?
    };
    let store = Arc::new(store);

    let session_id = uuid::Uuid::new_v4();
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).map_err(|err| {
        OtelImportFailure::env(
            "session_open_failed",
            format!("cannot open new session {session_id}: {err}"),
        )
    })?;
    let report = plan.emit(store.as_ref(), &mut graph)?;
    Ok(OtelImportOutcome {
        report,
        journal_dir,
    })
}

/// Renders an absolute path for display, falling back to the raw path.
fn otel_path_display(path: &Path) -> String {
    dunce::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Quotes a string for a POSIX shell when it contains special characters.
pub(crate) fn shell_quote(raw: &str) -> String {
    let safe = !raw.is_empty()
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:@%+=,".contains(c));
    if safe {
        raw.to_owned()
    } else {
        format!("'{}'", raw.replace('\'', r"'\''"))
    }
}

/// The exact command that exports the imported session as a bundle.
fn next_command(session_id: &str, journal_path: &str) -> String {
    format!(
        "akmon bundle export {session_id} --journal {} --output {}",
        shell_quote(journal_path),
        shell_quote(&format!("{session_id}.akmon"))
    )
}

/// Prints `value` as pretty JSON on stdout.
fn print_json<T: Serialize>(value: &T) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(value).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

/// Imports one OTLP/JSON GenAI trace into a fresh session in the local journal.
fn run_otel_import(args: &OtelImportArgs) -> ExitCode {
    let json = matches!(args.format, OtelImportFormat::Json);
    let fail = |failure: OtelImportFailure| -> ExitCode {
        if json {
            let _ = print_json(&OtelImportInfraError {
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                error: failure.message,
                category: failure.category.to_owned(),
            });
        } else {
            eprintln!("akmon: otel import: {}", failure.message);
        }
        ExitCode::from(failure.exit_code)
    };

    let options = ImportOptions {
        trace_id: args.trace_id.clone(),
        max_bytes: args.max_bytes,
    };
    let outcome = match import_trace_into_journal(&args.trace, args.journal.clone(), &options) {
        Ok(outcome) => outcome,
        Err(failure) => return fail(failure),
    };
    let report = outcome.report;
    let journal_path = otel_path_display(outcome.journal_dir.as_path());
    let sid = report.session_id.as_hyphenated().to_string();
    let next = next_command(&sid, &journal_path);

    if json {
        let body = OtelImportReportV1 {
            akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
            session_id: sid,
            capture_level: report.capture_level.as_str().to_owned(),
            content_slots_real: report.content_slots_real,
            content_slots_total: report.content_slots_total,
            provider_calls: report.provider_calls,
            tool_calls: report.tool_calls,
            failed_calls: report.failed_calls,
            turns_emitted: report.turns_emitted,
            turns_suppressed_no_content: report.turns_suppressed_no_content,
            user_messages_omitted: report.user_messages_omitted,
            non_text_parts_omitted: report.non_text_parts_omitted,
            duplicate_spans_dropped: report.duplicate_spans_dropped,
            spans_skipped: report.spans_skipped,
            structural_events_recorded: report.structural_events_recorded,
            semconv_version: report.semconv_version,
            source_semconv: report.source_semconv,
            source_form: report.source_form,
            trace_id: report.trace_id,
            journal_path,
            next_command: next,
        };
        if let Err(err) = print_json(&body) {
            return fail(OtelImportFailure::env(
                "io_error",
                format!("failed to render JSON output: {err}"),
            ));
        }
        return ExitCode::SUCCESS;
    }

    eprintln!("imported OTel GenAI trace into a new AGEF session");
    eprintln!(
        "  capture level: {} ({}/{} content slots captured)",
        report.capture_level.as_str(),
        report.content_slots_real,
        report.content_slots_total
    );
    match report.capture_level {
        CaptureLevel::Structural => eprintln!(
            "    note: metadata only; message content was NOT captured by the source telemetry"
        ),
        CaptureLevel::Partial => eprintln!(
            "    note: some provider/tool calls carry only metadata; content was partially captured"
        ),
        CaptureLevel::Full => {}
    }
    eprintln!(
        "  provider calls: {} | tool calls: {} | failed: {}",
        report.provider_calls, report.tool_calls, report.failed_calls
    );
    eprintln!(
        "  turns emitted: {} | suppressed (no content): {}",
        report.turns_emitted, report.turns_suppressed_no_content
    );
    if report.duplicate_spans_dropped > 0 {
        eprintln!(
            "  duplicate spans dropped: {}",
            report.duplicate_spans_dropped
        );
    }
    if report.structural_events_recorded > 0 {
        eprintln!(
            "  gen_ai events recorded as metadata: {}",
            report.structural_events_recorded
        );
    }
    eprintln!(
        "  semconv: source {} ({}), importer supports {}",
        report.source_semconv, report.source_form, report.semconv_version
    );
    eprintln!("  journal: {journal_path}");
    eprintln!("  session id: {sid}");
    eprintln!("next: {next}");
    println!("{sid}");
    ExitCode::SUCCESS
}

/// Runs one `akmon otel` subcommand.
pub fn run_otel(args: &OtelArgs) -> ExitCode {
    match &args.command {
        OtelCommands::Import(import_args) => run_otel_import(import_args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_leaves_safe_paths_and_quotes_others() {
        assert_eq!(shell_quote("/tmp/j-1"), "/tmp/j-1");
        assert_eq!(shell_quote("/tmp/my journal"), "'/tmp/my journal'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn next_command_is_exact() {
        assert_eq!(
            next_command("abc", "/j"),
            "akmon bundle export abc --journal /j --output abc.akmon"
        );
    }
}
