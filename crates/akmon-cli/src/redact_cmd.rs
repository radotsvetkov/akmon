//! `akmon redact`: write a sanitized derivative bundle with chosen objects replaced by sentinels.
//!
//! The source journal is never modified; the derivative is a valid AGEF bundle.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;

use akmon_bundle::{
    Manifest, Producer, SessionMetadata, WriteBundleOptions, sentinel_from_original,
    sentinel_to_canonical_cbor, write_bundle,
};
use akmon_journal::{
    AGEF_SPEC_VERSION, Event, EventKind, Hash, HashAlgorithm, ObjectStore, SessionGraph,
    digest_bytes, referenced_object_hashes_for_kind,
};
use akmon_query::{default_journal_dir, open_journal_read_only};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::{bundle_cmd, format_iso_utc, journal_errors, truncate_hash};

/// Output format for `akmon redact` status messages.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum RedactFormat {
    /// Human-readable status messages.
    #[default]
    Human,
    /// Machine-readable JSON status messages.
    Json,
}

/// Stable JSON shape for `akmon redact --format json`.
#[derive(Debug, Serialize, Deserialize)]
struct RedactReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// AGEF specification version written into the derivative bundle.
    agef_version: String,
    /// Source session UUID from the journal.
    source_session_id: String,
    /// Source terminal event hash before redaction rewrite.
    source_head: String,
    /// Derivative terminal event hash after rewrite.
    derivative_head: String,
    /// Event count in source session / derivative bundle.
    events_in_session: u64,
    /// Number of events whose content hash changed in derivative chain.
    events_rewritten_count: u64,
    /// Number of unique objects redacted.
    objects_redacted_count: u64,
    /// Per-redacted-object mapping to sentinel replacement.
    redacted_objects: Vec<RedactedObjectEntry>,
    /// Path to written derivative bundle.
    output_path: String,
    /// Written derivative bundle size in bytes.
    bundle_size_bytes: u64,
}

/// One redacted object mapping entry in `RedactReportV1`.
#[derive(Debug, Serialize, Deserialize)]
struct RedactedObjectEntry {
    /// Original object hash.
    original_hash: String,
    /// Replacement sentinel object hash.
    sentinel_hash: String,
    /// Original object byte length.
    original_size: u64,
}

/// JSON shape emitted when `akmon redact` cannot complete.
#[derive(Debug, Serialize, Deserialize)]
struct RedactError {
    /// CLI crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
    /// Invalid `--object` hash text when `category == invalid_object_hash`.
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid_object_hash: Option<String>,
    /// Missing-in-session object hash when `category == object_not_in_session`.
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_object_hash: Option<String>,
}

/// Arguments for `akmon redact`.
#[derive(Args, Debug, Clone)]
pub(crate) struct RedactArgs {
    /// Session UUID assigned at AgentSession construction.
    pub(crate) session_id: uuid::Uuid,
    /// Path where the sanitized derivative bundle file will be written.
    ///
    /// Required — no default to prevent accidental overwrite of source artifacts.
    #[arg(long)]
    pub(crate) output: PathBuf,
    /// Object hash to redact (hex, lowercase). Repeat for multiple objects.
    #[arg(long, required = true)]
    pub(crate) object: Vec<String>,
    /// Audit rationale recorded in each sentinel.
    #[arg(long)]
    pub(crate) reason: String,
    /// Path to the source journal directory.
    ///
    /// Defaults to per-user journal location (`$XDG_STATE_HOME/akmon/journal`).
    #[arg(long)]
    pub(crate) journal: Option<PathBuf>,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: RedactFormat,
}

pub(crate) fn run_redact(
    session_id: uuid::Uuid,
    output: PathBuf,
    object: Vec<String>,
    reason: String,
    journal: Option<PathBuf>,
    format: RedactFormat,
) -> ExitCode {
    let journal_dir = match journal {
        Some(path) => path,
        None => match default_journal_dir() {
            Ok(path) => path,
            Err(err) => {
                let msg = format!("cannot resolve default journal directory: {err}");
                emit_redact_error(format, "journal_not_found", msg, None, None);
                return ExitCode::from(3);
            }
        },
    };

    if output.exists() {
        emit_redact_error(
            format,
            "output_exists",
            format!("output path already exists: {}", output.display()),
            None,
            None,
        );
        return ExitCode::from(2);
    }

    let handle = match open_journal_read_only(journal_dir.as_path(), session_id) {
        Ok(h) => h,
        Err(err) => {
            let msg = format!(
                "cannot open journal {} for session {}: {err}",
                journal_dir.display(),
                session_id
            );
            let category = if msg.contains("session not found") {
                "session_not_found"
            } else {
                journal_errors::journal_open_category(&msg).unwrap_or("io_error")
            };
            emit_redact_error(format, category, msg, None, None);
            return ExitCode::from(3);
        }
    };

    let algorithm = handle.store.algorithm();
    let requested_hashes = match parse_requested_redact_hashes(algorithm, &object) {
        Ok(v) => v,
        Err(bad) => {
            emit_redact_error(
                format,
                "invalid_object_hash",
                format!("invalid --object hash '{bad}'"),
                Some(bad),
                None,
            );
            return ExitCode::from(2);
        }
    };

    let (history, source_head) = {
        let graph = handle
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = match graph.history() {
            Ok(h) => h,
            Err(err) => {
                emit_redact_error(
                    format,
                    "io_error",
                    format!("cannot read session history: {err}"),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
        };
        let source_head = match graph.head() {
            Ok(Some(h)) => h,
            Ok(None) => {
                emit_redact_error(
                    format,
                    "malformed_journal",
                    "malformed session: empty event graph (no head)".to_owned(),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
            Err(err) => {
                emit_redact_error(
                    format,
                    "io_error",
                    format!("cannot read session head: {err}"),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
        };
        (history, source_head)
    };

    let referenced: HashSet<Hash> = history
        .iter()
        .flat_map(|(_, ev)| referenced_object_hashes_for_kind(&ev.kind).into_iter())
        .collect();
    for requested in &requested_hashes {
        if !referenced.contains(requested) {
            emit_redact_error(
                format,
                "object_not_in_session",
                format!(
                    "requested --object hash is not referenced by source session: {}",
                    requested.to_hex()
                ),
                None,
                Some(requested.to_hex()),
            );
            return ExitCode::from(2);
        }
    }

    let mut replacement_map: HashMap<Hash, Hash> = HashMap::new();
    let mut sentinel_by_hash: HashMap<Hash, Vec<u8>> = HashMap::new();
    let mut redacted_entries: Vec<RedactedObjectEntry> = Vec::new();
    let now = time::OffsetDateTime::now_utc();
    for original_hash in &requested_hashes {
        let original_bytes = match handle.store.get(original_hash) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                emit_redact_error(
                    format,
                    "missing_object",
                    format!(
                        "requested object {} is referenced but missing from store",
                        original_hash.to_hex()
                    ),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
            Err(err) => {
                emit_redact_error(
                    format,
                    "io_error",
                    format!("cannot read object {}: {err}", original_hash.to_hex()),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
        };
        let marker =
            sentinel_from_original(original_hash, original_bytes.len() as u64, &reason, now);
        let sentinel_bytes = match sentinel_to_canonical_cbor(&marker) {
            Ok(v) => v,
            Err(err) => {
                emit_redact_error(
                    format,
                    "bundle_error",
                    format!(
                        "cannot serialize sentinel for {}: {err}",
                        original_hash.to_hex()
                    ),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
        };
        let sentinel_hash = digest_bytes(algorithm, sentinel_bytes.as_slice());
        replacement_map.insert(original_hash.clone(), sentinel_hash.clone());
        sentinel_by_hash.insert(sentinel_hash.clone(), sentinel_bytes);
        redacted_entries.push(RedactedObjectEntry {
            original_hash: original_hash.to_hex(),
            sentinel_hash: sentinel_hash.to_hex(),
            original_size: original_bytes.len() as u64,
        });
    }

    let mut rewritten_events: Vec<Event> = Vec::with_capacity(history.len());
    let mut rewritten_hashes: Vec<Hash> = Vec::with_capacity(history.len());
    let mut events_rewritten_count: u64 = 0;
    for (idx, (old_hash, old_event)) in history.iter().enumerate() {
        let (rewritten_kind, _) = rewrite_event_kind_hashes(&old_event.kind, &replacement_map);
        let mut parents = old_event.parents.clone();
        if idx > 0 {
            let prev = rewritten_hashes[idx - 1].clone();
            if parents.is_empty() {
                parents.push(prev);
            } else {
                parents[0] = prev;
            }
        }
        let rewritten_event = Event {
            parents,
            kind: rewritten_kind,
            emitted_at: old_event.emitted_at,
            sequence: old_event.sequence,
        };
        let rewritten_hash = match rewritten_event.content_hash(algorithm) {
            Ok(h) => h,
            Err(err) => {
                emit_redact_error(
                    format,
                    "bundle_error",
                    format!(
                        "cannot recompute event hash at sequence {}: {err}",
                        old_event.sequence
                    ),
                    None,
                    None,
                );
                return ExitCode::from(3);
            }
        };
        if rewritten_hash != *old_hash {
            events_rewritten_count += 1;
        }
        rewritten_events.push(rewritten_event);
        rewritten_hashes.push(rewritten_hash);
    }
    let Some(derivative_head) = rewritten_hashes.last().cloned() else {
        emit_redact_error(
            format,
            "malformed_journal",
            "malformed session: empty event graph (no events)".to_owned(),
            None,
            None,
        );
        return ExitCode::from(3);
    };

    let Some(start_event) = rewritten_events
        .iter()
        .find(|e| matches!(e.kind, EventKind::SessionStart { .. }))
    else {
        emit_redact_error(
            format,
            "malformed_journal",
            "malformed session: journal has no SessionStart event".to_owned(),
            None,
            None,
        );
        return ExitCode::from(3);
    };
    let Some(end_event) = rewritten_events
        .iter()
        .rev()
        .find(|e| matches!(e.kind, EventKind::SessionEnd { .. }))
    else {
        emit_redact_error(
            format,
            "malformed_journal",
            "malformed session: journal has no SessionEnd event".to_owned(),
            None,
            None,
        );
        return ExitCode::from(3);
    };

    let mut derivative_objects: HashMap<Hash, Vec<u8>> = HashMap::new();
    for event in &rewritten_events {
        for hash in referenced_object_hashes_for_kind(&event.kind) {
            if derivative_objects.contains_key(&hash) {
                continue;
            }
            if let Some(bytes) = sentinel_by_hash.get(&hash) {
                derivative_objects.insert(hash, bytes.clone());
                continue;
            }
            match handle.store.get(&hash) {
                Ok(Some(bytes)) => {
                    derivative_objects.insert(hash, bytes.to_vec());
                }
                Ok(None) => {
                    emit_redact_error(
                        format,
                        "missing_object",
                        format!(
                            "referenced object {} is missing from source store",
                            hash.to_hex()
                        ),
                        None,
                        None,
                    );
                    return ExitCode::from(3);
                }
                Err(err) => {
                    emit_redact_error(
                        format,
                        "io_error",
                        format!("cannot read object {}: {err}", hash.to_hex()),
                        None,
                        None,
                    );
                    return ExitCode::from(3);
                }
            }
        }
    }

    let manifest = Manifest {
        agef_version: AGEF_SPEC_VERSION.to_owned(),
        producer: Producer {
            name: "akmon".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        session: SessionMetadata {
            id: session_id.as_hyphenated().to_string(),
            head: derivative_head.to_hex(),
            created_at: format_iso_utc(
                start_event.emitted_at.unix_timestamp(),
                start_event.emitted_at.nanosecond(),
            ),
            ended_at: format_iso_utc(
                end_event.emitted_at.unix_timestamp(),
                end_event.emitted_at.nanosecond(),
            ),
        },
        hash_algorithm: algorithm.to_string(),
        object_count: derivative_objects.len() as u64,
        event_count: rewritten_events.len() as u64,
        signatures: None,
        operator_attestations: None,
        extra: BTreeMap::new(),
    };

    let mut file = match std::fs::File::create(&output) {
        Ok(f) => f,
        Err(err) => {
            emit_redact_error(
                format,
                "io_error",
                format!("cannot create bundle file {}: {err}", output.display()),
                None,
                None,
            );
            return ExitCode::from(3);
        }
    };
    if let Err(err) = write_bundle(
        &mut file,
        &manifest,
        &rewritten_events,
        &derivative_objects,
        &WriteBundleOptions::default(),
    ) {
        let _ = std::fs::remove_file(&output);
        emit_redact_error(
            format,
            "bundle_error",
            format!("bundle write failed: {err}"),
            None,
            None,
        );
        return ExitCode::from(3);
    }
    let size_bytes = match std::fs::metadata(&output) {
        Ok(m) => m.len(),
        Err(err) => {
            emit_redact_error(
                format,
                "io_error",
                format!("cannot stat bundle file {}: {err}", output.display()),
                None,
                None,
            );
            return ExitCode::from(3);
        }
    };

    match format {
        RedactFormat::Human => {
            eprintln!("redacted: source session {}", session_id.as_hyphenated());
            eprintln!("  events in session: {}", rewritten_events.len());
            eprintln!("  events rewritten: {events_rewritten_count}");
            eprintln!("  objects redacted: {}", redacted_entries.len());
            eprintln!("  source head: {}", truncate_hash(&source_head));
            eprintln!("  new head: {}", truncate_hash(&derivative_head));
            eprintln!(
                "  bundle: {}",
                bundle_cmd::bundle_export_output_display(output.as_path())
            );
            eprintln!(
                "  size: {}",
                bundle_cmd::format_bundle_byte_size(size_bytes)
            );
        }
        RedactFormat::Json => {
            let report = RedactReportV1 {
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                agef_version: AGEF_SPEC_VERSION.to_owned(),
                source_session_id: session_id.as_hyphenated().to_string(),
                source_head: source_head.to_hex(),
                derivative_head: derivative_head.to_hex(),
                events_in_session: rewritten_events.len() as u64,
                events_rewritten_count,
                objects_redacted_count: redacted_entries.len() as u64,
                redacted_objects: redacted_entries,
                output_path: bundle_cmd::bundle_export_output_display(output.as_path()),
                bundle_size_bytes: size_bytes,
            };
            if let Err(err) = print_redact_json_report(&report) {
                eprintln!("akmon: redact: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        }
    }
    ExitCode::SUCCESS
}

fn parse_requested_redact_hashes(
    algorithm: HashAlgorithm,
    values: &[String],
) -> Result<Vec<Hash>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let parsed = Hash::parse_hex(algorithm, value).map_err(|_| value.clone())?;
        if seen.insert(parsed.clone()) {
            out.push(parsed);
        }
    }
    Ok(out)
}

fn rewrite_hash(hash: &Hash, replacements: &HashMap<Hash, Hash>) -> (Hash, bool) {
    if let Some(replacement) = replacements.get(hash) {
        (replacement.clone(), true)
    } else {
        (hash.clone(), false)
    }
}

fn rewrite_event_kind_hashes(
    kind: &EventKind,
    replacements: &HashMap<Hash, Hash>,
) -> (EventKind, bool) {
    let mut changed = false;
    let rewritten = match kind {
        EventKind::SessionStart {
            cwd_hash,
            config_hash,
        } => {
            let (cwd_hash, c1) = rewrite_hash(cwd_hash, replacements);
            let (config_hash, c2) = rewrite_hash(config_hash, replacements);
            changed = c1 || c2;
            EventKind::SessionStart {
                cwd_hash,
                config_hash,
            }
        }
        EventKind::UserTurn { prompt_hash } => {
            let (prompt_hash, c1) = rewrite_hash(prompt_hash, replacements);
            changed = c1;
            EventKind::UserTurn { prompt_hash }
        }
        EventKind::ProviderCall {
            provider_id,
            attempts,
            stream_hash,
        } => {
            let mut rewritten_attempts = Vec::with_capacity(attempts.len());
            for attempt in attempts {
                let (request_hash, c1) = rewrite_hash(&attempt.request_hash, replacements);
                let (response_hash, c2) = match attempt.response_hash.as_ref() {
                    Some(h) => {
                        let (rewritten, c) = rewrite_hash(h, replacements);
                        (Some(rewritten), c)
                    }
                    None => (None, false),
                };
                let (attempt_stream_hash, c3) = match attempt.stream_hash.as_ref() {
                    Some(h) => {
                        let (rewritten, c) = rewrite_hash(h, replacements);
                        (Some(rewritten), c)
                    }
                    None => (None, false),
                };
                changed |= c1 || c2 || c3;
                rewritten_attempts.push(akmon_journal::AttemptRecord {
                    attempt_number: attempt.attempt_number,
                    started_at: attempt.started_at,
                    ended_at: attempt.ended_at,
                    status: attempt.status.clone(),
                    request_hash,
                    response_hash,
                    stream_hash: attempt_stream_hash,
                    error_message: attempt.error_message.clone(),
                });
            }
            let (stream_hash, c4) = match stream_hash.as_ref() {
                Some(h) => {
                    let (rewritten, c) = rewrite_hash(h, replacements);
                    (Some(rewritten), c)
                }
                None => (None, false),
            };
            changed |= c4;
            EventKind::ProviderCall {
                provider_id: provider_id.clone(),
                attempts: rewritten_attempts,
                stream_hash,
            }
        }
        EventKind::ToolCall {
            tool_id,
            input_hash,
            output_hash,
            side_effects_hash,
        } => {
            let (input_hash, c1) = rewrite_hash(input_hash, replacements);
            let (output_hash, c2) = rewrite_hash(output_hash, replacements);
            let (side_effects_hash, c3) = match side_effects_hash.as_ref() {
                Some(h) => {
                    let (rewritten, c) = rewrite_hash(h, replacements);
                    (Some(rewritten), c)
                }
                None => (None, false),
            };
            changed = c1 || c2 || c3;
            EventKind::ToolCall {
                tool_id: tool_id.clone(),
                input_hash,
                output_hash,
                side_effects_hash,
            }
        }
        EventKind::RetrievalCall {
            index_id,
            query_hash,
            results_hash,
        } => {
            let (query_hash, c1) = rewrite_hash(query_hash, replacements);
            let (results_hash, c2) = rewrite_hash(results_hash, replacements);
            changed = c1 || c2;
            EventKind::RetrievalCall {
                index_id: index_id.clone(),
                query_hash,
                results_hash,
            }
        }
        EventKind::PermissionGate {
            policy_id,
            decision,
            context_hash,
        } => {
            let (context_hash, c1) = rewrite_hash(context_hash, replacements);
            changed = c1;
            EventKind::PermissionGate {
                policy_id: policy_id.clone(),
                decision: decision.clone(),
                context_hash,
            }
        }
        EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        } => {
            let (message_hash, c1) = rewrite_hash(message_hash, replacements);
            let (tool_calls_hash, c2) = match tool_calls_hash.as_ref() {
                Some(h) => {
                    let (rewritten, c) = rewrite_hash(h, replacements);
                    (Some(rewritten), c)
                }
                None => (None, false),
            };
            changed = c1 || c2;
            EventKind::AssistantTurn {
                message_hash,
                tool_calls_hash,
            }
        }
        EventKind::SessionEnd { summary_hash } => {
            let (summary_hash, c1) = match summary_hash.as_ref() {
                Some(h) => {
                    let (rewritten, c) = rewrite_hash(h, replacements);
                    (Some(rewritten), c)
                }
                None => (None, false),
            };
            changed = c1;
            EventKind::SessionEnd { summary_hash }
        }
    };
    (rewritten, changed)
}

fn print_redact_json_report(report: &RedactReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn emit_redact_error(
    format: RedactFormat,
    category: &str,
    error: String,
    invalid_object_hash: Option<String>,
    missing_object_hash: Option<String>,
) {
    match format {
        RedactFormat::Human => eprintln!("akmon: redact: {error}"),
        RedactFormat::Json => {
            let body = RedactError {
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                error,
                category: category.to_owned(),
                invalid_object_hash,
                missing_object_hash,
            };
            match serde_json::to_string_pretty(&body) {
                Ok(json) => println!("{json}"),
                Err(err) => eprintln!("akmon: redact: failed to render JSON error output: {err}"),
            }
        }
    }
}
