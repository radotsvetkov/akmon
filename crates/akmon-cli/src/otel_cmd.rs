//! `akmon otel` — import OpenTelemetry GenAI traces into AGEF sessions (Item 9.1).
//!
//! `akmon otel import` ingests an OTLP/JSON OpenTelemetry GenAI trace (semconv
//! v1.37.0+ structured form) into a fresh AGEF session in the local journal, so
//! the producer-agnostic result composes with `akmon bundle export` / `sign` /
//! `verify` and the standalone `agef-verify` tool.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use akmon_journal::{HashAlgorithm, RedbObjectStore, RedbSessionGraph};
use akmon_otel::{OtelImportError, import_otel_genai};
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
Parses a single OTLP `ExportTraceServiceRequest` JSON document using either the semconv >= \
v1.37.0 structured GenAI attributes or the supported legacy (<= v1.36) message-event forms \
(`gen_ai.system.message` / `gen_ai.user.message` / `gen_ai.assistant.message` / \
`gen_ai.tool.message` / `gen_ai.choice`, reduced to the same structured content), maps its spans \
onto AGEF events in a deterministic order, stores every referenced content object, and appends \
the events to a fresh session in the local journal. \
The produced session is a valid AGEF merkle chain, so it composes directly with the rest of the \
toolchain:\n\
  akmon otel import trace.json --journal ./j\n\
  akmon bundle export <session-id> --journal ./j --output session.akmon\n\
  akmon bundle sign session.akmon --key signer.pk8\n\
  akmon bundle verify session.akmon --verify-key signer.pub.hex   # or: agef-verify\n\n\
Capture-level honesty: GenAI message/tool content is opt-in in the OpenTelemetry conventions and \
is frequently absent. When real content is present it is hashed directly and the import reports \
capture_level=full; when only structural metadata is present the required hash slots are filled \
with self-describing labeled objects and the import reports capture_level=structural (message \
content was NOT captured by the source telemetry). The capture level is baked into the signed \
session head.\n\n\
Examples:\n\
  akmon otel import trace.json\n\
  akmon otel import trace.json --journal ~/audit/journal\n\
  akmon otel import trace.json --format json\n\n\
Exit codes:\n\
  0 — trace imported successfully\n\
  2 — usage error (unparseable trace, unrecognized legacy gen_ai.* event, multiple sessions, or empty trace)\n\
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
    /// Capture level: `full` (real content captured) or `structural` (metadata only).
    capture_level: String,
    /// Number of provider-call events emitted.
    provider_calls: u64,
    /// Number of tool-call events emitted.
    tool_calls: u64,
    /// Number of user/assistant turn events emitted (with real content).
    turns_emitted: u64,
    /// Number of turn events suppressed because only metadata (no real content) was available.
    turns_suppressed_no_content: u64,
    /// The pinned semconv version this import targeted.
    semconv_version: String,
    /// Resolved journal directory the session was written into.
    journal_path: String,
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

/// Renders an absolute path for display, falling back to the raw path.
fn otel_path_display(path: &std::path::Path) -> String {
    dunce::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

/// Prints a `category`/`error` infra-error object as pretty JSON.
fn print_otel_import_json_error(category: &str, error: String) -> std::io::Result<()> {
    let body = OtelImportInfraError {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        error,
        category: category.to_owned(),
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

/// Prints a successful import report as pretty JSON.
fn print_otel_import_json_report(report: &OtelImportReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

/// Maps an [`OtelImportError`] to its `(category, exit_code)`.
///
/// Parse / legacy / multi-session / empty-trace are usage errors (exit 2); a
/// journal/object-store failure is an environment error (exit 3).
fn otel_import_error_mapping(err: &OtelImportError) -> (&'static str, u8) {
    match err {
        OtelImportError::Parse(_) => ("parse_error", 2),
        OtelImportError::LegacySemconvUnsupported => ("legacy_semconv_unsupported", 2),
        OtelImportError::MultipleSessions => ("multiple_sessions", 2),
        OtelImportError::EmptyTrace => ("empty_trace", 2),
        OtelImportError::Journal(_) => ("journal_error", 3),
    }
}

/// Imports one OTLP/JSON GenAI trace into a fresh session in the local journal.
fn run_otel_import(trace: PathBuf, journal: Option<PathBuf>, format: OtelImportFormat) -> ExitCode {
    let json = matches!(format, OtelImportFormat::Json);
    let fail = |category: &str, msg: String, code: u8| -> ExitCode {
        if json {
            let _ = print_otel_import_json_error(category, msg);
        } else {
            eprintln!("akmon: otel import: {msg}");
        }
        ExitCode::from(code)
    };

    // (1) Read the trace file bytes (IO error -> exit 3).
    let bytes = match std::fs::read(&trace) {
        Ok(b) => b,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot read trace file {}: {err}", trace.display()),
                3,
            );
        }
    };

    // (2) Resolve the journal dir (explicit --journal, else per-user default).
    let journal_dir = match journal {
        Some(path) => path,
        None => match default_journal_dir() {
            Ok(path) => path,
            Err(err) => {
                return fail(
                    "journal_not_found",
                    format!("cannot resolve default journal directory: {err}"),
                    3,
                );
            }
        },
    };

    if let Err(err) = std::fs::create_dir_all(journal_dir.as_path()) {
        return fail(
            "journal_access",
            format!(
                "cannot create journal directory {}: {err}",
                journal_dir.display()
            ),
            3,
        );
    }

    // (3) Open or create the RedbObjectStore at the journal path (mirror bundle import).
    let journal_db = journal_db_path(journal_dir.as_path());
    let store = if journal_db.is_file() {
        match RedbObjectStore::open(journal_db.as_path()) {
            Ok(s) => Arc::new(s),
            Err(err) => {
                return fail(
                    "journal_access",
                    format!("cannot open journal store {}: {err}", journal_db.display()),
                    3,
                );
            }
        }
    } else {
        match RedbObjectStore::create(journal_db.as_path(), HashAlgorithm::Sha256) {
            Ok(s) => Arc::new(s),
            Err(err) => {
                return fail(
                    "journal_access",
                    format!(
                        "cannot create journal store {}: {err}",
                        journal_db.display()
                    ),
                    3,
                );
            }
        }
    };

    // (4) Fresh session id + new empty session graph.
    let session_id = uuid::Uuid::new_v4();
    let mut graph = match RedbSessionGraph::open_new(Arc::clone(&store), session_id) {
        Ok(g) => g,
        Err(err) => {
            return fail(
                "session_open_failed",
                format!("cannot open new session {session_id}: {err}"),
                3,
            );
        }
    };

    // (5) Import the trace.
    let report = match import_otel_genai(&bytes, store.as_ref(), &mut graph) {
        Ok(r) => r,
        Err(err) => {
            let (category, code) = otel_import_error_mapping(&err);
            return fail(category, err.to_string(), code);
        }
    };

    // (6) Report.
    let journal_path = otel_path_display(journal_dir.as_path());
    let capture_level = report.capture_level.as_str();
    let is_structural = matches!(report.capture_level, akmon_otel::CaptureLevel::Structural);
    match format {
        OtelImportFormat::Json => {
            let body = OtelImportReportV1 {
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                session_id: report.session_id.as_hyphenated().to_string(),
                capture_level: capture_level.to_owned(),
                provider_calls: report.provider_calls,
                tool_calls: report.tool_calls,
                turns_emitted: report.turns_emitted,
                turns_suppressed_no_content: report.turns_suppressed_no_content,
                semconv_version: report.semconv_version.clone(),
                journal_path,
            };
            if let Err(err) = print_otel_import_json_report(&body) {
                return fail(
                    "io_error",
                    format!("failed to render JSON output: {err}"),
                    3,
                );
            }
        }
        OtelImportFormat::Human => {
            let sid = report.session_id.as_hyphenated().to_string();
            eprintln!("imported OTel GenAI trace into a new AGEF session");
            eprintln!("  capture level: {capture_level}");
            if is_structural {
                eprintln!(
                    "    note: metadata only; message content was NOT captured by the source telemetry"
                );
            }
            eprintln!("  session id: {sid}");
            eprintln!("  provider calls: {}", report.provider_calls);
            eprintln!("  tool calls: {}", report.tool_calls);
            eprintln!("  turns emitted: {}", report.turns_emitted);
            eprintln!(
                "  turns suppressed (no content): {}",
                report.turns_suppressed_no_content
            );
            eprintln!("  semconv version: {}", report.semconv_version);
            eprintln!("  journal: {journal_path}");
            eprintln!("  next step: akmon bundle export {sid} --journal {journal_path}");
        }
    }
    ExitCode::SUCCESS
}

/// Runs one `akmon otel` subcommand.
pub fn run_otel(args: &OtelArgs) -> ExitCode {
    match &args.command {
        OtelCommands::Import(import_args) => run_otel_import(
            import_args.trace.clone(),
            import_args.journal.clone(),
            import_args.format,
        ),
    }
}
