//! Akmon CLI — project discovery, optional `AKMON.md`, and headless `--task` runs.

mod audit_cmd;
mod cli_forward;
mod cli_project;
mod config_cmd;
mod doctor_cmd;
mod evidence_cmd;
mod export_cmd;
mod import_cmd;
mod policy_cmd;
mod scout_cmd;
mod session_index;
mod session_transcript;
mod slo_cmd;
mod spec_cmd;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
#[cfg(feature = "semantic-index")]
use std::time::Duration;

use akmon_config::AkmonGlobalConfig;
use akmon_core::{
    AgentConfig, AgentError, AgentEvent, AuditEvent, EvidenceArtifact, EvidenceAudit,
    EvidencePolicy, EvidenceToolCall, EvidenceTools, EvidenceVerification, InteractivePolicyReply,
    McpServerConfig, PolicyEngine, PolicyEngineMode, PolicyVerdict, REPLAY_HASH_ALGORITHM,
    ReplayMetadata, RunReliabilityMetrics, Sandbox, verify_audit_jsonl, write_audit_jsonl,
    write_evidence_json,
};
use akmon_journal::{ObjectStore, SessionGraph};
use akmon_models::{
    LlmConnectConfig, LlmProvider, Message, MessageRole, ProviderError, ProviderResolutionTrace,
};
use akmon_query::{
    AgentSession, SessionRunExit, SpawnSubagentTool, SubagentRuntime, SubagentToolFactory,
    ToolCallSummary, default_journal_dir, open_default_journal_handle, open_journal_read_only,
    write_handoff_file,
};
#[cfg(feature = "semantic-index")]
use akmon_tools::SemanticSearchTool;
use akmon_tools::{
    ApplyPatchTool, AskFollowupTool, EditTool, GitTool, ListDirectoryTool, MemoryWriteTool,
    PatchTool, ReadFileTool, ReadSpecTool, SearchTool, ShellTool, TodoWriteTool, WebFetchTool,
    WriteFileTool, WriteSpecTool, discover_mcp_tools,
};
use akmon_tui::TuiLaunchConfig;
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
#[cfg(feature = "semantic-index")]
use fastembed::{TextEmbedding, TextInitOptions};
use serde::Serialize;
use serde_json::json;
#[cfg(feature = "semantic-index")]
use tokio::sync::RwLock;
use tokio::sync::mpsc;

/// Builds a semantic index in the background and writes `index_path`, then fills `slot`.
#[cfg(feature = "semantic-index")]
async fn semantic_index_background_build(
    project_root: PathBuf,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    sandbox: Arc<Sandbox>,
    index_path: PathBuf,
    slot: Arc<RwLock<Option<akmon_index::RepoIndex>>>,
) {
    let indexer = akmon_index::Indexer::default();
    match indexer
        .build_index(&project_root, embedder, sandbox.as_ref())
        .await
    {
        Ok(idx) => {
            match index_path.parent() {
                Some(parent) => {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("akmon: failed to create .akmon dir: {e}");
                    }
                }
                None => {
                    eprintln!(
                        "akmon: index save path has no parent (unexpected): {}",
                        index_path.display()
                    );
                }
            }
            match akmon_index::save_index(&idx, &index_path) {
                Ok(()) => {
                    eprintln!(
                        "akmon: index saved to .akmon/index.bin ({} files, {} chunks)",
                        idx.file_count, idx.chunk_count
                    );
                }
                Err(e) => eprintln!("akmon: index save FAILED: {e}"),
            }
            *slot.write().await = Some(idx);
        }
        Err(e) => {
            eprintln!("akmon: warning: semantic index build failed: {e}");
        }
    }
}

/// Runs [`semantic_index_background_build`] on a dedicated OS thread with its own current-thread
/// Tokio runtime so index work is not cancelled when `#[tokio::main]` shuts down.
///
/// The caller should [`std::thread::JoinHandle::join`] the handle before process exit so
/// `index.bin` can be written reliably.
#[cfg(feature = "semantic-index")]
fn spawn_semantic_index_os_thread(
    project_root: PathBuf,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    sandbox: Arc<Sandbox>,
    index_path: PathBuf,
    slot: Arc<RwLock<Option<akmon_index::RepoIndex>>>,
) -> Option<std::thread::JoinHandle<()>> {
    match std::thread::Builder::new()
        .name("akmon-index-build".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                eprintln!("akmon: failed to build index runtime");
                return;
            };
            rt.block_on(async move {
                semantic_index_background_build(project_root, embedder, sandbox, index_path, slot)
                    .await;
            });
        }) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("akmon: failed to spawn index thread: {e}");
            None
        }
    }
}

/// Gives a short background build time to finish so the first model turn may see an index.
#[cfg(feature = "semantic-index")]
async fn poll_index_ready_up_to_3s(slot: &Arc<RwLock<Option<akmon_index::RepoIndex>>>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if slot.read().await.is_some() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Returns true if `path` is an existing `.git` directory or gitdir file (worktrees).
fn git_working_tree_marker_present(git_path: &Path) -> bool {
    match std::fs::symlink_metadata(git_path) {
        Ok(m) => m.is_dir() || m.is_file(),
        Err(_) => false,
    }
}

/// At most this many directories are checked when walking upward for `.git` (current dir first).
const GIT_ROOT_MAX_DIR_CHECKS: usize = 5;

/// Walks upward from `root` for at most [`GIT_ROOT_MAX_DIR_CHECKS`] directories.
///
/// If the environment variable `AKMON_DEBUG_GIT` is set (any value), prints each directory checked
/// and whether a `.git` marker was found to stderr (for troubleshooting discovery).
fn walk_up_for_git_limited(mut root: PathBuf, max_dir_checks: usize) -> Option<PathBuf> {
    let debug_git = std::env::var_os("AKMON_DEBUG_GIT").is_some();
    for _ in 0..max_dir_checks {
        let git_path = root.join(".git");
        let found = git_working_tree_marker_present(&git_path);
        if debug_git {
            eprintln!(
                "akmon: debug git: dir={} git_marker_present={} .git_path={}",
                root.display(),
                found,
                git_path.display()
            );
        }
        if found {
            return Some(root);
        }
        root = root.parent()?.to_path_buf();
    }
    None
}

fn push_git_walk_start(candidates: &mut Vec<PathBuf>, p: PathBuf) {
    if !candidates.iter().any(|c| c == &p) {
        candidates.push(p);
    }
}

/// Walks upward from `start` looking for a `.git` file or directory, at most
/// [`GIT_ROOT_MAX_DIR_CHECKS`] levels per candidate start path. Returns the directory that
/// contains `.git`, or [`None`] if none is found within that depth.
///
/// Tries several starting paths (`dunce::canonicalize`, [`Path::canonicalize`], and the logical
/// absolute path from [`std::env::current_dir`]) so a repo is found when only one representation
/// resolves correctly.
pub(crate) fn find_git_project_root(start: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(c) = dunce::canonicalize(start) {
        push_git_walk_start(&mut candidates, c);
    }

    if let Ok(c) = start.canonicalize() {
        push_git_walk_start(&mut candidates, c);
    }

    let logical = if start.is_absolute() {
        start.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(start),
            Err(_) => start.to_path_buf(),
        }
    };
    push_git_walk_start(&mut candidates, logical);

    for root in candidates {
        if let Some(found) = walk_up_for_git_limited(root, GIT_ROOT_MAX_DIR_CHECKS) {
            return Some(found);
        }
    }
    None
}

/// Reads `AKMON.md` from `project_root` when the file exists.
///
/// Warns when the file is large: it is reinjected on every model call, so oversized files can
/// dominate input cost despite prompt caching.
const AKMON_MD_MAX_CHARS: usize = 2000;

fn load_akmon_md(project_root: &Path) -> std::io::Result<Option<String>> {
    let path = project_root.join("AKMON.md");
    if path.is_file() {
        let content = std::fs::read_to_string(path)?;
        if content.len() > AKMON_MD_MAX_CHARS {
            tracing::warn!(
                akmon_md_chars = content.len(),
                akmon_md_tokens_estimate = content.len() / 4,
                "AKMON.md is large; files over {} chars (~500+ tokens) add more cost than they save — consider trimming",
                AKMON_MD_MAX_CHARS
            );
        }
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

fn load_dossier_system_block(path: &Path) -> Result<String, String> {
    let dossier = scout_cmd::load_dossier(path)?;
    Ok(scout_cmd::dossier_prompt_block(&dossier))
}

fn merge_akmon_with_dossier(
    akmon_md: Option<String>,
    dossier_block: Option<String>,
) -> Option<String> {
    match (akmon_md, dossier_block) {
        (Some(mut md), Some(block)) => {
            md.push_str("\n\n");
            md.push_str(&block);
            Some(md)
        }
        (None, Some(block)) => Some(block),
        (md, None) => md,
    }
}

/// How human-readable versus machine-readable the process should be on stdout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    /// Stream assistant text and tool progress to the terminal (default).
    #[default]
    Text,
    /// Suppress streaming output; emit a single JSON object on stdout when the run finishes.
    Json,
}

/// Output format for `akmon verify`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum VerifyFormat {
    /// Human-readable summary and optional detail output.
    #[default]
    Human,
    /// Machine-readable JSON output for CI automation.
    Json,
}

/// Output format for `akmon inspect`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum InspectFormat {
    /// Human-readable summary and optional detail output.
    #[default]
    Human,
    /// Machine-readable JSON output for automation.
    Json,
}

/// Output format for `akmon bundle export`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum BundleExportFormat {
    /// Human-readable status messages.
    #[default]
    Human,
    /// Machine-readable JSON status messages.
    Json,
}

/// Display mode for resolved binary object content in `akmon inspect`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum BinaryMode {
    /// Show binary metadata only (`<binary, N bytes, hash: ...>`).
    #[default]
    Meta,
    /// Show a truncated hexadecimal preview.
    Hex,
    /// Show a truncated base64 preview.
    Base64,
}

/// Stable JSON shape for `akmon verify --format json`.
#[derive(Debug, Serialize, serde::Deserialize)]
struct VerifyReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// AGEF specification version implemented by the journal substrate.
    agef_version: String,
    /// Hyphenated session UUID.
    session_id: String,
    /// Resolved journal directory used for verification.
    journal_path: String,
    /// Number of events walked.
    events_checked: u32,
    /// Number of object references checked.
    objects_checked: u32,
    /// True when verification found no violations.
    passed: bool,
    /// Stable list of verification checks attempted.
    checks_performed: Vec<akmon_journal::VerifyCheck>,
    /// Flattened violations with stable categories.
    violations: Vec<VerifyViolation>,
}

/// One machine-readable verification violation.
#[derive(Debug, Serialize, serde::Deserialize)]
struct VerifyViolation {
    /// Stable category identifier.
    category: String,
    /// Event hash in hex when applicable.
    event_hash: Option<String>,
    /// Object hash in hex when applicable.
    object_hash: Option<String>,
    /// Human-readable explanation.
    message: String,
}

/// JSON shape emitted when verification cannot run (journal/session/infrastructure errors).
#[derive(Debug, Serialize, serde::Deserialize)]
struct VerifyError {
    /// CLI crate version that produced this error.
    akmon_version: String,
    /// Stable infrastructure error category.
    category: String,
    /// Human-readable error description.
    error: String,
}

/// Stable JSON shape for `akmon inspect --format json`.
#[derive(Debug, Serialize, serde::Deserialize)]
struct InspectReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// AGEF specification version implemented by the journal substrate.
    agef_version: String,
    /// Hyphenated session UUID.
    session_id: String,
    /// Resolved journal directory used for inspection.
    journal_path: String,
    /// Session events in sequence order.
    events: Vec<InspectEvent>,
}

/// One inspected event in machine-readable format.
#[derive(Debug, Serialize, serde::Deserialize)]
struct InspectEvent {
    /// Monotonic per-session sequence.
    sequence: u64,
    /// Event content hash (hex).
    event_hash: String,
    /// Parent event hashes (hex).
    parent_hashes: Vec<String>,
    /// Event timestamp (ISO 8601 UTC).
    emitted_at: String,
    /// Kind-specific payload.
    kind: InspectEventKind,
}

/// Kind-specific event payload for `InspectEvent`.
#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InspectEventKind {
    /// Session start payload.
    SessionStart {
        cwd_hash: String,
        config_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        config_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        config_size: Option<u64>,
    },
    /// User turn payload.
    UserTurn {
        prompt_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_size: Option<u64>,
    },
    /// Provider call payload.
    ProviderCall {
        provider_id: String,
        attempts: Vec<InspectAttempt>,
        stream_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stream_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stream_size: Option<u64>,
    },
    /// Tool call payload.
    ToolCall {
        tool_id: String,
        input_hash: String,
        output_hash: String,
        side_effects_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        side_effects_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        side_effects_size: Option<u64>,
    },
    /// Retrieval call payload.
    RetrievalCall {
        index_id: String,
        query_hash: String,
        results_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        query_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        query_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        results_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        results_size: Option<u64>,
    },
    /// Permission gate payload.
    PermissionGate {
        policy_id: String,
        decision: String,
        context_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_size: Option<u64>,
    },
    /// Assistant turn payload.
    AssistantTurn {
        message_hash: String,
        tool_calls_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls_size: Option<u64>,
    },
    /// Session end payload.
    SessionEnd {
        summary_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary_text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary_size: Option<u64>,
    },
}

/// Provider attempt details for JSON inspect output.
#[derive(Debug, Serialize, serde::Deserialize)]
struct InspectAttempt {
    /// 1-indexed attempt number.
    attempt_number: u32,
    /// Attempt status.
    status: String,
    /// Attempt start timestamp (ISO 8601 UTC).
    started_at: String,
    /// Attempt end timestamp (ISO 8601 UTC).
    ended_at: String,
    /// Request payload hash.
    request_hash: String,
    /// Response payload hash when present.
    response_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_size: Option<u64>,
    /// Stream transcript hash when present.
    stream_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_size: Option<u64>,
    /// Human-readable error message when present.
    error_message: Option<String>,
}

/// JSON shape emitted when inspect cannot read the session/journal.
#[derive(Debug, Serialize, serde::Deserialize)]
struct InspectError {
    /// CLI crate version that produced this error.
    akmon_version: String,
    /// Stable infrastructure error category.
    category: String,
    /// Human-readable error description.
    error: String,
}

fn verify_report_v1(
    session_id: uuid::Uuid,
    journal_path: &Path,
    report: &akmon_journal::VerificationReport,
) -> VerifyReportV1 {
    let mut violations = Vec::new();

    violations.extend(report.missing_objects.iter().map(|missing| {
        VerifyViolation {
            category: "missing_object".to_owned(),
            event_hash: missing
                .referenced_by_event
                .as_ref()
                .map(akmon_journal::Hash::to_hex),
            object_hash: Some(missing.object_hash.to_hex()),
            message: match missing.referenced_by_event.as_ref() {
                Some(event_hash) => {
                    format!(
                        "Object referenced by event {} not in store",
                        event_hash.to_hex()
                    )
                }
                None => "Object referenced but not in store".to_owned(),
            },
        }
    }));

    violations.extend(
        report
            .object_hash_mismatches
            .iter()
            .map(|hash| VerifyViolation {
                category: "object_hash_mismatch".to_owned(),
                event_hash: None,
                object_hash: Some(hash.to_hex()),
                message: "Object bytes do not match hash".to_owned(),
            }),
    );

    violations.extend(report.hash_mismatches.iter().map(|hash| VerifyViolation {
        category: "event_hash_mismatch".to_owned(),
        event_hash: Some(hash.to_hex()),
        object_hash: None,
        message: "Event hash does not match recomputed value".to_owned(),
    }));

    violations.extend(report.broken_parent_links.iter().map(
        |(event_hash, expected_parent_hash)| VerifyViolation {
            category: "parent_chain".to_owned(),
            event_hash: Some(event_hash.to_hex()),
            object_hash: None,
            message: format!(
                "Event parent does not match prior event hash (expected parent {})",
                expected_parent_hash.to_hex()
            ),
        },
    ));

    violations.extend(
        report
            .sequence_violations
            .iter()
            .map(|seq| VerifyViolation {
                category: "sequence".to_owned(),
                event_hash: None,
                object_hash: None,
                message: format!("Event sequence number incorrect: {seq}"),
            }),
    );

    if let Some((stored, computed)) = report.head_mismatch.as_ref() {
        violations.push(VerifyViolation {
            category: "head_mismatch".to_owned(),
            event_hash: None,
            object_hash: None,
            message: format!(
                "Stored head does not match terminal event hash (stored {}, terminal {})",
                stored.to_hex(),
                computed.to_hex()
            ),
        });
    }

    match report.session_end_count {
        0 => violations.push(VerifyViolation {
            category: "session_end_missing".to_owned(),
            event_hash: None,
            object_hash: None,
            message: "SessionEnd event is missing".to_owned(),
        }),
        n if n > 1 => violations.push(VerifyViolation {
            category: "session_end_duplicate".to_owned(),
            event_hash: None,
            object_hash: None,
            message: format!("SessionEnd appears multiple times (count={n})"),
        }),
        1 if !report.session_end_is_terminal => violations.push(VerifyViolation {
            category: "session_end_not_terminal".to_owned(),
            event_hash: None,
            object_hash: None,
            message: "SessionEnd is not the terminal event".to_owned(),
        }),
        _ => {}
    }

    let journal_path =
        dunce::canonicalize(journal_path).unwrap_or_else(|_| journal_path.to_path_buf());
    VerifyReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: akmon_journal::AGEF_SPEC_VERSION.to_owned(),
        session_id: session_id.to_string(),
        journal_path: journal_path.display().to_string(),
        events_checked: u32::try_from(report.events_checked).unwrap_or(u32::MAX),
        objects_checked: u32::try_from(report.objects_checked).unwrap_or(u32::MAX),
        passed: report.is_clean(),
        checks_performed: report.checks_performed.clone(),
        violations,
    }
}

fn verify_check_name(check: akmon_journal::VerifyCheck) -> &'static str {
    match check {
        akmon_journal::VerifyCheck::ParentChain => "parent chain",
        akmon_journal::VerifyCheck::Sequence => "sequence",
        akmon_journal::VerifyCheck::EventHashRecompute => "event hash recompute",
        akmon_journal::VerifyCheck::ObjectPresence => "object presence",
        akmon_journal::VerifyCheck::ObjectByteRehash => "object byte re-hash",
        akmon_journal::VerifyCheck::HeadConsistency => "head consistency",
        akmon_journal::VerifyCheck::SessionEndInvariants => "SessionEnd invariants",
    }
}

fn inspect_attempt_status_name(status: &akmon_journal::AttemptStatus) -> String {
    match status {
        akmon_journal::AttemptStatus::Success => "success".to_owned(),
        akmon_journal::AttemptStatus::RateLimited => "rate_limited".to_owned(),
        akmon_journal::AttemptStatus::NetworkError => "network_error".to_owned(),
        akmon_journal::AttemptStatus::ServerError => "server_error".to_owned(),
        akmon_journal::AttemptStatus::ClientError => "client_error".to_owned(),
        akmon_journal::AttemptStatus::Cancelled => "cancelled".to_owned(),
        akmon_journal::AttemptStatus::Other(other) => format!("other:{other}"),
    }
}

enum ContentClass {
    Text(String),
    Binary(usize),
    Empty,
}

#[derive(Clone, Default)]
struct ResolvedContent {
    text: Option<String>,
    size: Option<u64>,
}

fn resolve_object<S: ObjectStore>(store: &S, hash: &akmon_journal::Hash) -> Option<Vec<u8>> {
    store
        .get(hash)
        .ok()
        .and_then(|bytes| bytes.map(|b| b.to_vec()))
}

fn classify_content(bytes: &[u8]) -> ContentClass {
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.is_empty() => ContentClass::Text(text.to_owned()),
        Ok(_) => ContentClass::Empty,
        Err(_) => ContentClass::Binary(bytes.len()),
    }
}

fn resolved_content<S: ObjectStore>(
    store: &S,
    hash: &akmon_journal::Hash,
    resolve: bool,
) -> ResolvedContent {
    if !resolve {
        return ResolvedContent::default();
    }
    let Some(bytes) = resolve_object(store, hash) else {
        return ResolvedContent {
            text: None,
            size: None,
        };
    };
    let size = Some(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    match classify_content(&bytes) {
        ContentClass::Text(text) => ResolvedContent {
            text: Some(text),
            size,
        },
        ContentClass::Binary(_) | ContentClass::Empty => ResolvedContent { text: None, size },
    }
}

fn inspect_event_kind<S: ObjectStore>(
    kind: &akmon_journal::EventKind,
    store: &S,
    resolve: bool,
) -> InspectEventKind {
    match kind {
        akmon_journal::EventKind::SessionStart {
            cwd_hash,
            config_hash,
        } => {
            let cwd = resolved_content(store, cwd_hash, resolve);
            let config = resolved_content(store, config_hash, resolve);
            InspectEventKind::SessionStart {
                cwd_hash: cwd_hash.to_hex(),
                config_hash: config_hash.to_hex(),
                cwd_text: cwd.text,
                cwd_size: cwd.size,
                config_text: config.text,
                config_size: config.size,
            }
        }
        akmon_journal::EventKind::UserTurn { prompt_hash } => {
            let prompt = resolved_content(store, prompt_hash, resolve);
            InspectEventKind::UserTurn {
                prompt_hash: prompt_hash.to_hex(),
                prompt_text: prompt.text,
                prompt_size: prompt.size,
            }
        }
        akmon_journal::EventKind::ProviderCall {
            provider_id,
            attempts,
            stream_hash,
        } => {
            let stream_resolved = stream_hash
                .as_ref()
                .map(|h| resolved_content(store, h, resolve))
                .unwrap_or_default();
            InspectEventKind::ProviderCall {
                provider_id: provider_id.clone(),
                attempts: attempts
                    .iter()
                    .map(|attempt| {
                        let request = resolved_content(store, &attempt.request_hash, resolve);
                        let response = attempt
                            .response_hash
                            .as_ref()
                            .map(|h| resolved_content(store, h, resolve))
                            .unwrap_or_default();
                        let stream = attempt
                            .stream_hash
                            .as_ref()
                            .map(|h| resolved_content(store, h, resolve))
                            .unwrap_or_default();
                        InspectAttempt {
                            attempt_number: attempt.attempt_number,
                            status: inspect_attempt_status_name(&attempt.status),
                            started_at: format_iso_utc(
                                attempt.started_at.unix_timestamp(),
                                attempt.started_at.nanosecond(),
                            ),
                            ended_at: format_iso_utc(
                                attempt.ended_at.unix_timestamp(),
                                attempt.ended_at.nanosecond(),
                            ),
                            request_hash: attempt.request_hash.to_hex(),
                            request_text: request.text,
                            request_size: request.size,
                            response_hash: attempt
                                .response_hash
                                .as_ref()
                                .map(akmon_journal::Hash::to_hex),
                            response_text: response.text,
                            response_size: response.size,
                            stream_hash: attempt
                                .stream_hash
                                .as_ref()
                                .map(akmon_journal::Hash::to_hex),
                            stream_text: stream.text,
                            stream_size: stream.size,
                            error_message: attempt.error_message.clone(),
                        }
                    })
                    .collect(),
                stream_hash: stream_hash.as_ref().map(akmon_journal::Hash::to_hex),
                stream_text: stream_resolved.text,
                stream_size: stream_resolved.size,
            }
        }
        akmon_journal::EventKind::ToolCall {
            tool_id,
            input_hash,
            output_hash,
            side_effects_hash,
        } => {
            let input = resolved_content(store, input_hash, resolve);
            let output = resolved_content(store, output_hash, resolve);
            let side_effects = side_effects_hash
                .as_ref()
                .map(|h| resolved_content(store, h, resolve))
                .unwrap_or_default();
            InspectEventKind::ToolCall {
                tool_id: tool_id.clone(),
                input_hash: input_hash.to_hex(),
                output_hash: output_hash.to_hex(),
                side_effects_hash: side_effects_hash.as_ref().map(akmon_journal::Hash::to_hex),
                input_text: input.text,
                input_size: input.size,
                output_text: output.text,
                output_size: output.size,
                side_effects_text: side_effects.text,
                side_effects_size: side_effects.size,
            }
        }
        akmon_journal::EventKind::RetrievalCall {
            index_id,
            query_hash,
            results_hash,
        } => {
            let query = resolved_content(store, query_hash, resolve);
            let results = resolved_content(store, results_hash, resolve);
            InspectEventKind::RetrievalCall {
                index_id: index_id.clone(),
                query_hash: query_hash.to_hex(),
                results_hash: results_hash.to_hex(),
                query_text: query.text,
                query_size: query.size,
                results_text: results.text,
                results_size: results.size,
            }
        }
        akmon_journal::EventKind::PermissionGate {
            policy_id,
            decision,
            context_hash,
        } => {
            let context = resolved_content(store, context_hash, resolve);
            InspectEventKind::PermissionGate {
                policy_id: policy_id.clone(),
                decision: decision.clone(),
                context_hash: context_hash.to_hex(),
                context_text: context.text,
                context_size: context.size,
            }
        }
        akmon_journal::EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        } => {
            let message = resolved_content(store, message_hash, resolve);
            let tool_calls = tool_calls_hash
                .as_ref()
                .map(|h| resolved_content(store, h, resolve))
                .unwrap_or_default();
            InspectEventKind::AssistantTurn {
                message_hash: message_hash.to_hex(),
                tool_calls_hash: tool_calls_hash.as_ref().map(akmon_journal::Hash::to_hex),
                message_text: message.text,
                message_size: message.size,
                tool_calls_text: tool_calls.text,
                tool_calls_size: tool_calls.size,
            }
        }
        akmon_journal::EventKind::SessionEnd { summary_hash } => {
            let summary = summary_hash
                .as_ref()
                .map(|h| resolved_content(store, h, resolve))
                .unwrap_or_default();
            InspectEventKind::SessionEnd {
                summary_hash: summary_hash.as_ref().map(akmon_journal::Hash::to_hex),
                summary_text: summary.text,
                summary_size: summary.size,
            }
        }
    }
}

fn inspect_report_v1<S: ObjectStore>(
    session_id: uuid::Uuid,
    journal_path: &Path,
    store: &S,
    resolve: bool,
    history: &[(akmon_journal::Hash, akmon_journal::Event)],
) -> InspectReportV1 {
    let journal_path =
        dunce::canonicalize(journal_path).unwrap_or_else(|_| journal_path.to_path_buf());
    let events = history
        .iter()
        .map(|(hash, event)| InspectEvent {
            sequence: event.sequence,
            event_hash: hash.to_hex(),
            parent_hashes: event
                .parents
                .iter()
                .map(akmon_journal::Hash::to_hex)
                .collect(),
            emitted_at: format_iso_utc(
                event.emitted_at.unix_timestamp(),
                event.emitted_at.nanosecond(),
            ),
            kind: inspect_event_kind(&event.kind, store, resolve),
        })
        .collect();
    InspectReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: akmon_journal::AGEF_SPEC_VERSION.to_owned(),
        session_id: session_id.to_string(),
        journal_path: journal_path.display().to_string(),
        events,
    }
}

fn print_inspect_json_report(report: &InspectReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_inspect_json_error(category: &'static str, error: String) -> std::io::Result<()> {
    let body = InspectError {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        category: category.to_owned(),
        error,
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn inspect_error_category(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("session not found") {
        "session_not_found"
    } else if lower.contains("redb open failed") || lower.contains("no such file or directory") {
        "journal_not_found"
    } else if lower.contains("history") {
        "history_read_error"
    } else {
        "inspect_infrastructure_error"
    }
}

fn print_verify_json_report(report: &VerifyReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_verify_json_error(category: &'static str, error: String) -> std::io::Result<()> {
    let body = VerifyError {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        category: category.to_owned(),
        error,
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn verify_error_category(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("session not found") {
        "session_not_found"
    } else if lower.contains("redb open failed") || lower.contains("no such file or directory") {
        "journal_not_found"
    } else if lower.contains("hash algorithm mismatch") {
        "hash_algorithm_mismatch"
    } else {
        "verify_infrastructure_error"
    }
}

/// Aggregated token counters for `--output json` (Anthropic cache fields are zero when unused).
#[derive(Debug, Serialize)]
struct RunUsageSummary {
    /// Sum of provider `input_tokens` across completions in this run.
    total_input_tokens: u32,
    /// Sum of `cache_read_input_tokens` (prompt-cache hits) when the backend reports them.
    total_cache_read_tokens: u32,
    /// Sum of `output_tokens` across completions in this run.
    total_output_tokens: u32,
}

/// Why a headless run stopped (JSON `--output json`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // Interrupted reserved for future signal handling
enum ExitReason {
    Completed,
    MaxTurns,
    BudgetLimit,
    Error,
    Interrupted,
}

/// Resolved run summary for `--output json` (stable field names for automation).
#[derive(Debug, Serialize)]
struct RunReport {
    /// Session identifier ([`AgentConfig::session_id`] as hyphenated `Uuid`).
    session_id: String,
    /// `"success"` or `"error"`.
    status: &'static str,
    /// Machine-readable stop reason (headless integrations).
    exit_reason: ExitReason,
    /// Full assistant reply text accumulated from stream deltas.
    result: String,
    /// Tool completions in chronological order.
    tool_calls: Vec<ToolCallSummary>,
    /// Present only when `status` is `"error"`; JSON `null` on success.
    error: Option<String>,
    /// Path passed to [`write_audit_jsonl`] for this run (default or `--audit-log`).
    audit_log_path: String,
    /// Token totals for this run (Ollama typically leaves cache fields at zero).
    usage: RunUsageSummary,
    /// Estimated cumulative USD (heuristic; zero when unknown or local).
    cost_usd: f64,
    /// Prompt-cache read tokens (duplicate of `usage` for CI consumers).
    cache_read_tokens: u64,
    /// Sandbox-relative paths touched by successful writes/edits this run.
    files_written: Vec<String>,
    /// Deterministic replay metadata for forensic reproducibility.
    replay_metadata: ReplayMetadata,
    /// Reliability/SLO counters for this run.
    reliability_metrics: RunReliabilityMetrics,
    /// Provider routing trace for the effective CLI model/config (explainability only).
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_resolution: Option<ProviderResolutionTrace>,
}

fn replay_metadata_for_report<S, G>(session: &AgentSession<S, G>) -> ReplayMetadata
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    if let Some(m) = session.replay_metadata() {
        return m.clone();
    }
    let provider = session.provider_arc();
    ReplayMetadata {
        hash_algorithm: REPLAY_HASH_ALGORITHM.to_string(),
        provider_name: provider.name().to_string(),
        model_id: provider.completion_model_id().to_string(),
        session_id: session.session_id().to_string(),
        policy_hash: "0".repeat(64),
        config_hash: "0".repeat(64),
        tool_registry_hash: "0".repeat(64),
        prompt_assembly_hash: None,
    }
}

/// Single JSON object on stdout for `--output json` when exiting before any agent session exists.
fn print_json_early_error_and_exit(error: String) -> ! {
    let error_report = json!({
        "session_id": "",
        "status": "error",
        "result": "",
        "tool_calls": [],
        "error": error,
        "exit_reason": "error",
        "cost_usd": 0.0,
        "files_written": [],
        "cache_read_tokens": 0,
        "reliability_metrics": RunReliabilityMetrics::default(),
    });
    println!("{error_report}");
    std::process::exit(2);
}

/// Configuration / setup failure: stderr in text mode, JSON on stdout when `--output json`.
fn exit_early_config_error(
    cli: &Cli,
    error: String,
    index_thread: Option<&mut Option<std::thread::JoinHandle<()>>>,
    text_exit_code: i32,
) -> ! {
    if let Some(slot) = index_thread
        && let Some(h) = slot.take()
    {
        eprintln!("akmon: waiting for index to finish building...");
        let _ = h.join();
    }
    if cli.output == OutputFormat::Json {
        print_json_early_error_and_exit(error);
    }
    eprintln!("akmon: {error}");
    std::process::exit(text_exit_code);
}

/// [`resolve_resume_session_id`] failure (text mode prints two lines).
fn exit_resume_session_error(cli: &Cli, e: String) -> ! {
    if cli.output == OutputFormat::Json {
        print_json_early_error_and_exit(format!("{e}\nStart a new session: akmon"));
    }
    eprintln!("akmon: {e}");
    eprintln!("Start a new session: akmon");
    std::process::exit(1);
}

/// Command-line interface for the Akmon agent binary.
#[derive(Parser, Debug)]
#[command(
    name = "akmon",
    version,
    about = "Local-first AI coding agent. Runs with Ollama (local) or Anthropic API. All actions are audited."
)]
pub(crate) struct Cli {
    /// Optional subcommand (`chat` is equivalent to omitting `--task`).
    #[command(subcommand)]
    command: Option<Commands>,
    /// Task string for a headless agent run. When omitted, Akmon opens the interactive TUI.
    #[arg(short = 't', long = "task", global = true)]
    task: Option<String>,
    /// Model name: Ollama tag (e.g. llama3.2), or a Claude id if `ANTHROPIC_API_KEY` / `--anthropic-key` is set.
    #[arg(long = "model", default_value = "llama3.2", global = true)]
    model: String,
    /// Anthropic API key; defaults to the `ANTHROPIC_API_KEY` environment variable when unset.
    #[arg(
        long = "anthropic-key",
        env = "ANTHROPIC_API_KEY",
        hide_env_values = true,
        global = true,
        help = "Anthropic API key (falls back to ANTHROPIC_API_KEY env var)"
    )]
    anthropic_key: Option<String>,
    /// OpenRouter API key (`OPENROUTER_API_KEY`).
    #[arg(
        long = "openrouter-key",
        env = "OPENROUTER_API_KEY",
        hide_env_values = true,
        global = true
    )]
    openrouter_key: Option<String>,
    /// OpenAI API key (`OPENAI_API_KEY`).
    #[arg(
        long = "openai-key",
        env = "OPENAI_API_KEY",
        hide_env_values = true,
        global = true
    )]
    openai_key: Option<String>,
    /// Groq API key (`GROQ_API_KEY`).
    #[arg(
        long = "groq-key",
        env = "GROQ_API_KEY",
        hide_env_values = true,
        global = true
    )]
    groq_key: Option<String>,
    /// Azure OpenAI deployment URL (…/deployments/NAME/chat/completions).
    #[arg(long = "azure-endpoint", env = "AZURE_OPENAI_ENDPOINT", global = true)]
    azure_endpoint: Option<String>,
    #[arg(
        long = "azure-key",
        env = "AZURE_OPENAI_API_KEY",
        hide_env_values = true,
        global = true
    )]
    azure_key: Option<String>,
    /// Azure `api-version` query parameter (default `2024-02-01`).
    #[arg(
        long = "azure-api-version",
        default_value = "2024-02-01",
        global = true
    )]
    azure_api_version: String,
    /// Use Amazon Bedrock (reads `AWS_*` credentials from the environment).
    #[arg(long = "bedrock", global = true)]
    bedrock: bool,
    /// AWS region for Bedrock.
    #[arg(
        long = "aws-region",
        env = "AWS_DEFAULT_REGION",
        default_value = "us-east-1",
        global = true
    )]
    aws_region: String,
    /// Custom OpenAI-compatible API base (no `/chat/completions` suffix).
    #[arg(long = "openai-compatible-url", global = true)]
    openai_compatible_url: Option<String>,
    #[arg(long = "openai-compatible-key", hide_env_values = true, global = true)]
    openai_compatible_key: Option<String>,
    /// Base URL for the Ollama HTTP API (ignored when using Anthropic).
    #[arg(
        long = "ollama-url",
        default_value = "http://localhost:11434",
        global = true
    )]
    ollama_url: String,
    /// Auto-approve read-only tools only; writes and `shell` still require confirmation.
    #[arg(short = 'y', long = "yes", global = true)]
    yes: bool,
    /// `text`: stream tokens to the terminal; `json`: print one session summary object at the end.
    #[arg(
        long = "output",
        value_name = "FORMAT",
        default_value = "text",
        value_enum
    )]
    output: OutputFormat,
    /// JSON Lines audit file path (default: `<project>/.akmon/audit/<session_id>.jsonl`).
    #[arg(long = "audit-log", value_name = "PATH")]
    audit_log: Option<PathBuf>,
    /// Evidence artifact output path (default: `<project>/.akmon/evidence/<session_id>.json`).
    #[arg(long = "evidence-path", value_name = "PATH")]
    evidence_path: Option<PathBuf>,
    /// Select built-in policy profile (`dev`, `staging`, `prod`).
    #[arg(long = "policy-profile", value_enum, global = true)]
    policy_profile: Option<policy_cmd::PolicyProfileArg>,
    /// Add a policy pack file (repeatable).
    #[arg(long = "policy-pack", value_name = "PATH", action = clap::ArgAction::Append, global = true)]
    policy_pack: Vec<PathBuf>,
    /// Highest-precedence policy override file.
    #[arg(long = "policy-override", value_name = "PATH", global = true)]
    policy_override: Option<PathBuf>,
    /// Glob pattern allowed for the `shell` tool (argv-style commands only). Repeat for multiple patterns.
    #[arg(long = "shell-allow", value_name = "PATTERN")]
    shell_allow: Vec<String>,
    /// Enable the `web_fetch` tool. Disabled by default. When enabled, the agent can fetch public URLs. Internal and private network addresses are always blocked.
    #[arg(long = "web-fetch")]
    web_fetch: bool,
    /// Auto-approve `web_fetch` requests to public URLs (use with `--web-fetch` and `--yes`). SSRF protection still applies. `WriteFile` and `shell` still require confirmation.
    #[arg(long = "yes-web")]
    yes_web: bool,
    /// Connect to an MCP server at this URL and register all tools it lists (repeatable for multiple servers).
    #[arg(long = "mcp-server", value_name = "URL")]
    mcp_server: Vec<String>,
    /// Build or load a semantic index of the project; registers the `semantic_search` tool. Cache: `.akmon/index.bin`.
    #[arg(long = "index", global = true)]
    index: bool,
    /// After each successful `edit` or `write_file`, run `git add` and `git commit` with an auto-generated message (requires a git repo).
    #[arg(long = "auto-commit", global = true)]
    auto_commit: bool,
    /// Analyze the codebase and produce a written plan. The agent uses read-only tools only; the plan is saved under `.akmon/plans/`. Run again without `--plan` to implement.
    #[arg(long = "plan", global = true)]
    plan: bool,
    /// Use architect mode: plan with `--planner-model` (or config), then implement with `--model`.
    #[arg(long = "architect", global = true)]
    architect: bool,
    /// Model id for the planning phase when using `--architect`. Overrides `[architect] planner_model` in `~/.akmon/config.toml`; default `llama3.2`.
    #[arg(long = "planner-model", value_name = "MODEL", global = true)]
    planner_model: Option<String>,
    /// Resume the last session for this project directory (uses `~/.akmon/last_session.json`).
    #[arg(
        short = 'c',
        long = "continue",
        global = true,
        conflicts_with = "resume_session"
    )]
    continue_last: bool,
    /// Resume a specific session id (full UUID or unique `*.json` prefix under `~/.akmon/sessions/`).
    #[arg(
        short = 's',
        long = "session",
        global = true,
        conflicts_with = "continue_last"
    )]
    resume_session: Option<String>,
    /// Display name for this session (status line / JSON tooling).
    #[arg(short = 'n', long = "name", global = true)]
    session_name: Option<String>,
    /// Stop after estimated spend reaches this USD amount (headless only; ignored for Ollama).
    #[arg(long = "max-budget-usd", global = true, value_name = "USD")]
    max_budget_usd: Option<f64>,
    /// Extra directories merged into the sandbox (in addition to the project root). Repeatable.
    #[arg(long = "add-dir", global = true, value_name = "DIR", action = clap::ArgAction::Append)]
    add_dirs: Vec<PathBuf>,
    /// Optional scout dossier JSON to inject into prompt context.
    #[arg(long = "dossier", global = true, value_name = "PATH")]
    dossier: Option<PathBuf>,
    /// Model to try on repeated HTTP 429/529 before giving up (headless only).
    #[arg(long = "fallback-model", global = true, value_name = "MODEL")]
    fallback_model: Option<String>,
}

/// Top-level `akmon` subcommands.
#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Interactive full-screen terminal UI (same as `akmon` without `--task`).
    Chat,
    /// Analyze the current project and generate `AKMON.md` using the configured model.
    Init,
    /// Create a new project directory with starter files and `AKMON.md`.
    New(cli_project::NewCmd),
    /// Manage `~/.akmon/config.toml` (models, keys, MCP).
    Config(config_cmd::ConfigArgs),
    /// Provider operability diagnostics and remediation hints.
    Doctor(doctor_cmd::DoctorArgs),
    /// Verify audit JSONL chain integrity.
    Audit(audit_cmd::AuditArgs),
    /// Verify evidence artifact integrity.
    Evidence(evidence_cmd::EvidenceArgs),
    /// Verify reliability metrics against SLO thresholds.
    Slo(slo_cmd::SloArgs),
    /// Policy profile/pack management and effective view.
    Policy(policy_cmd::PolicyArgs),
    /// Generate a bounded, read-only project context dossier.
    Scout(scout_cmd::ScoutArgs),
    /// Structured spec workflow under `.akmon/specs/<feature>/`.
    Spec(spec_cmd::SpecCmd),
    /// Synthesize `AKMON.md` from other tools' context files (Claude, Cursor, …).
    Import(import_cmd::ImportArgs),
    /// Write `AKMON.md` into another tool's expected paths (`--all` or `--tool`).
    Export(export_cmd::ExportArgs),
    /// Bundle operations: export and import AGEF bundles.
    #[command(long_about = "Bundle operations: export and import AGEF bundles.\n\n\
Bundles are portable artifacts containing a complete session graph plus all referenced objects. \
They are produced by `akmon bundle export` and consumed by `akmon bundle import`. \
The bundle format is AGEF-compliant (see github.com/radotsvetkov/agef).")]
    Bundle(BundleArgs),
    /// Verify a session's integrity (chain, hashes, object closure).
    #[command(
        long_about = "Verify the on-disk journal for the given session ID. Checks parent chain, \
sequence, event hashes, and object integrity per AGEF Section 13.\n\n\
Example:\n\
  akmon verify 550e8400-e29b-41d4-a716-446655440000\n\n\
Exit codes:\n\
  0 — verification passed\n\
  1 — verification failed (see output for violations)\n\
  2 — usage error\n\
  3 — I/O or environment error"
    )]
    Verify {
        /// Session UUID assigned at AgentSession construction.
        session_id: uuid::Uuid,
        /// Path to the journal directory. Defaults to per-user journal location ($XDG_STATE_HOME/akmon/journal).
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Output format: human (default) or json.
        #[arg(long, default_value = "human")]
        format: VerifyFormat,
        /// Print per-violation detail.
        #[arg(long)]
        verbose: bool,
    },
    /// Inspect a session's events and contents.
    #[command(
        long_about = "Inspect a session's events and contents from the on-disk\n\
journal. Shows the event timeline (SessionStart, UserTurn,\n\
ProviderCall, ToolCall, PermissionGate, AssistantTurn,\n\
SessionEnd) with kind-specific fields.\n\n\
Default human output is summary-style. Use --verbose for full\n\
detail (all hashes, attempt records, metadata). Use --resolve\n\
to display referenced object content (prompt text, message\n\
text, tool input/output) instead of just hashes.\n\n\
Examples:\n\
  akmon inspect 550e8400-e29b-41d4-a716-446655440000\n\
  akmon inspect 550e8400-... --verbose\n\
  akmon inspect 550e8400-... --resolve --binary hex\n\
  akmon inspect 550e8400-... --format json\n\n\
Exit codes:\n\
  0 — session displayed\n\
  1 — (reserved; not currently emitted by inspect)\n\
  2 — usage error (e.g., --binary without --resolve)\n\
  3 — I/O or environment error (journal/session not found)"
    )]
    Inspect {
        /// Session UUID assigned at AgentSession construction.
        session_id: uuid::Uuid,
        /// Path to the journal directory. Defaults to per-user journal location ($XDG_STATE_HOME/akmon/journal).
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Output format: human (default) or json.
        #[arg(long, default_value = "human")]
        format: InspectFormat,
        /// Print full event detail (all hashes, attempt records, metadata).
        #[arg(long)]
        verbose: bool,
        /// Resolve referenced object hashes and display content.
        #[arg(long)]
        resolve: bool,
        /// Display mode for non-UTF-8 resolved content. Requires --resolve for `hex`/`base64`.
        #[arg(long, default_value = "meta")]
        binary: BinaryMode,
    },
}

/// Arguments for `akmon bundle`.
#[derive(Args, Debug, Clone)]
struct BundleArgs {
    /// Bundle command to execute.
    #[command(subcommand)]
    command: BundleCommands,
}

/// Nested bundle subcommands.
#[derive(Subcommand, Debug, Clone)]
enum BundleCommands {
    /// Export a session as an AGEF bundle.
    #[command(long_about = "Export a session as an AGEF bundle.\n\n\
Reads the named session from the on-disk journal and writes a self-contained .akmon archive \
(tar.zst per AGEF v0.1.1) at the specified path.\n\n\
Examples:\n\
  akmon bundle export 550e8400-e29b-41d4-a716-446655440000\n\
  akmon bundle export 550e8400-... --output ~/audit/q3.akmon\n\
  akmon bundle export 550e8400-... --format json\n\n\
Exit codes:\n\
  0 — bundle written successfully\n\
  1 — (reserved; not currently emitted)\n\
  2 — usage error (e.g., output path is a directory)\n\
  3 — I/O or environment error (journal/session not found)")]
    Export(BundleExportArgs),
}

/// Arguments for `akmon bundle export`.
#[derive(Args, Debug, Clone)]
struct BundleExportArgs {
    /// Session UUID assigned at AgentSession construction.
    session_id: uuid::Uuid,
    /// Path where the bundle file will be written.
    ///
    /// If omitted, defaults to `<session-id>.akmon` in the current directory.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Path to the journal directory.
    ///
    /// Defaults to per-user journal location (`$XDG_STATE_HOME/akmon/journal`).
    #[arg(long)]
    journal: Option<PathBuf>,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    format: BundleExportFormat,
}

fn run_bundle_export(
    session_id: uuid::Uuid,
    output: Option<PathBuf>,
    journal: Option<PathBuf>,
    format: BundleExportFormat,
) -> ExitCode {
    eprintln!(
        "bundle export: session_id={session_id} output={output:?} journal={journal:?} format={format:?}"
    );
    eprintln!("(layer 1 stub — export logic in layer 2)");
    ExitCode::SUCCESS
}

fn run_verify(
    session_id: uuid::Uuid,
    journal: Option<PathBuf>,
    format: VerifyFormat,
    verbose: bool,
) -> ExitCode {
    let journal_dir = match journal {
        Some(path) => path,
        None => match default_journal_dir() {
            Ok(path) => path,
            Err(err) => {
                if matches!(format, VerifyFormat::Json) {
                    let _ = print_verify_json_error(
                        "verify_infrastructure_error",
                        format!("cannot resolve default journal directory: {err}"),
                    );
                } else {
                    eprintln!("akmon: verify: cannot resolve default journal directory: {err}");
                }
                return ExitCode::from(3);
            }
        },
    };

    let handle = match open_journal_read_only(journal_dir.as_path(), session_id) {
        Ok(h) => h,
        Err(err) => {
            let msg = format!(
                "cannot open journal {} for session {}: {err}",
                journal_dir.display(),
                session_id
            );
            if matches!(format, VerifyFormat::Json) {
                let _ = print_verify_json_error(verify_error_category(&msg), msg);
            } else {
                eprintln!("akmon: verify: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    let graph = handle
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let report = match graph.verify() {
        Ok(r) => r,
        Err(err) => {
            let msg = format!("verification failed with journal error: {err}");
            if matches!(format, VerifyFormat::Json) {
                let _ = print_verify_json_error(verify_error_category(&msg), msg);
            } else {
                eprintln!("akmon: verify: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    match format {
        VerifyFormat::Json => {
            let body = verify_report_v1(session_id, journal_dir.as_path(), &report);
            if let Err(err) = print_verify_json_report(&body) {
                eprintln!("akmon: verify: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        }
        VerifyFormat::Human => {
            if report.is_clean() {
                eprintln!("verified: session {session_id}");
                eprintln!("  events checked: {}", report.events_checked);
                eprintln!("  objects checked: {}", report.objects_checked);
                eprintln!("  SessionEnd: present and terminal");
                if verbose {
                    eprintln!();
                    eprintln!("  checks performed:");
                    for check in &report.checks_performed {
                        match check {
                            akmon_journal::VerifyCheck::ObjectPresence
                            | akmon_journal::VerifyCheck::ObjectByteRehash => {
                                eprintln!(
                                    "    - {}: ok ({})",
                                    verify_check_name(*check),
                                    report.objects_checked
                                );
                            }
                            _ => eprintln!("    - {}: ok", verify_check_name(*check)),
                        }
                    }
                }
            } else {
                eprintln!("verification failed: session {session_id}");
                eprintln!("  events checked: {}", report.events_checked);
                eprintln!("  objects checked: {}", report.objects_checked);
                eprintln!();
                eprintln!("  violations:");
                if !verbose {
                    eprintln!("    - missing objects: {}", report.missing_objects.len());
                    eprintln!(
                        "    - object hash mismatches: {}",
                        report.object_hash_mismatches.len()
                    );
                    eprintln!(
                        "    - event hash mismatches: {}",
                        report.hash_mismatches.len()
                    );
                    eprintln!(
                        "    - parent chain breaks: {}",
                        report.broken_parent_links.len()
                    );
                    eprintln!(
                        "    - sequence violations: {}",
                        report.sequence_violations.len()
                    );
                    eprintln!("    - head mismatch: {}", report.head_mismatch.is_some());
                } else {
                    eprintln!(
                        "    missing objects ({}): {}",
                        report.missing_objects.len(),
                        if report.missing_objects.is_empty() {
                            "none"
                        } else {
                            ""
                        }
                    );
                    for missing in &report.missing_objects {
                        match missing.referenced_by_event.as_ref() {
                            Some(event_hash) => {
                                eprintln!(
                                    "      - {} (referenced by event {})",
                                    missing.object_hash.to_hex(),
                                    event_hash.to_hex()
                                );
                            }
                            None => eprintln!("      - {}", missing.object_hash.to_hex()),
                        }
                    }
                    eprintln!();

                    eprintln!(
                        "    object hash mismatches ({}): {}",
                        report.object_hash_mismatches.len(),
                        if report.object_hash_mismatches.is_empty() {
                            "none"
                        } else {
                            ""
                        }
                    );
                    for hash in &report.object_hash_mismatches {
                        eprintln!("      - {}", hash.to_hex());
                    }
                    eprintln!();

                    eprintln!(
                        "    event hash mismatches ({}): {}",
                        report.hash_mismatches.len(),
                        if report.hash_mismatches.is_empty() {
                            "none"
                        } else {
                            ""
                        }
                    );
                    for hash in &report.hash_mismatches {
                        eprintln!("      - {}", hash.to_hex());
                    }
                    eprintln!();

                    eprintln!(
                        "    parent chain breaks ({}): {}",
                        report.broken_parent_links.len(),
                        if report.broken_parent_links.is_empty() {
                            "none"
                        } else {
                            ""
                        }
                    );
                    for (event_hash, expected_parent) in &report.broken_parent_links {
                        eprintln!(
                            "      - event {} expected parent {}",
                            event_hash.to_hex(),
                            expected_parent.to_hex()
                        );
                    }
                    eprintln!();

                    eprintln!(
                        "    sequence violations ({}): {}",
                        report.sequence_violations.len(),
                        if report.sequence_violations.is_empty() {
                            "none"
                        } else {
                            ""
                        }
                    );
                    for seq in &report.sequence_violations {
                        eprintln!("      - sequence={seq}");
                    }
                    eprintln!();

                    if let Some((stored, computed)) = report.head_mismatch.as_ref() {
                        eprintln!(
                            "    head mismatch: true (stored {} terminal {})",
                            stored.to_hex(),
                            computed.to_hex()
                        );
                    } else {
                        eprintln!("    head mismatch: false");
                    }
                }
                let session_end_summary = match report.session_end_count {
                    0 => "missing".to_owned(),
                    1 if report.session_end_is_terminal => "present and terminal".to_owned(),
                    1 => "not terminal".to_owned(),
                    n => format!("duplicate (count={n})"),
                };
                eprintln!("    - SessionEnd: {session_end_summary}");
            }
        }
    }

    if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn truncate_hash(hash: &akmon_journal::Hash) -> String {
    let hex = hash.to_hex();
    if hex.len() <= 8 {
        format!("{hex}...")
    } else {
        format!("{}...", &hex[..8])
    }
}

fn format_hash_full(hash: &akmon_journal::Hash) -> String {
    hash.to_hex()
}

fn format_optional_hash_full(hash: Option<&akmon_journal::Hash>) -> String {
    hash.map_or_else(|| "none".to_owned(), format_hash_full)
}

const RESOLVE_TEXT_MAX_BYTES: usize = 10 * 1024;
const RESOLVE_TEXT_PREVIEW_MAX_LINES: usize = 5;
const RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES: usize = 1024;
const RESOLVE_BINARY_HEX_MAX_BYTES: usize = 64;
const RESOLVE_BINARY_BASE64_MAX_CHARS: usize = 128;

fn format_hex_preview(bytes: &[u8], max_bytes: usize) -> String {
    let preview_len = bytes.len().min(max_bytes);
    let preview = bytes[..preview_len]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > max_bytes {
        format!(
            "{preview}... (truncated, {} more bytes)",
            bytes.len() - max_bytes
        )
    } else {
        preview
    }
}

fn format_base64_preview(bytes: &[u8], max_chars: usize) -> String {
    use base64::Engine;
    let full = base64::engine::general_purpose::STANDARD.encode(bytes);
    if full.len() > max_chars {
        format!(
            "{}... (truncated, {} more chars)",
            &full[..max_chars],
            full.len() - max_chars
        )
    } else {
        full
    }
}

fn format_resolved_human(
    hash: &akmon_journal::Hash,
    bytes: Option<Vec<u8>>,
    binary: BinaryMode,
) -> Vec<String> {
    let Some(bytes) = bytes else {
        return vec!["  | <unresolved>".to_owned()];
    };
    match classify_content(&bytes) {
        ContentClass::Empty => vec!["  | <empty>".to_owned()],
        ContentClass::Binary(size) => {
            let mut out = vec![format!(
                "  | <binary, {size} bytes, hash: {}>",
                truncate_hash(hash)
            )];
            match binary {
                BinaryMode::Meta => {}
                BinaryMode::Hex => out.push(format!(
                    "  | {}",
                    format_hex_preview(&bytes, RESOLVE_BINARY_HEX_MAX_BYTES)
                )),
                BinaryMode::Base64 => out.push(format!(
                    "  | {}",
                    format_base64_preview(&bytes, RESOLVE_BINARY_BASE64_MAX_CHARS)
                )),
            }
            out
        }
        ContentClass::Text(text) => {
            let lines: Vec<&str> = text.lines().collect();
            if lines.is_empty() {
                return vec!["  | <empty>".to_owned()];
            }
            if text.len() > RESOLVE_TEXT_MAX_BYTES || lines.len() > RESOLVE_TEXT_PREVIEW_MAX_LINES {
                let shown = lines
                    .iter()
                    .take(RESOLVE_TEXT_PREVIEW_MAX_LINES)
                    .map(|line| {
                        if line.len() > RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES {
                            format!(
                                "  | {}... (truncated, full hash: {})",
                                &line[..RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES],
                                hash.to_hex()
                            )
                        } else {
                            format!("  | {line}")
                        }
                    })
                    .collect::<Vec<_>>();
                let more = lines.len().saturating_sub(RESOLVE_TEXT_PREVIEW_MAX_LINES);
                let mut out = shown;
                out.push(format!(
                    "  | ... ({more} more lines, full content via --format json)"
                ));
                out
            } else {
                lines
                    .iter()
                    .map(|line| {
                        if line.len() > RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES {
                            format!(
                                "  | {}... (truncated, full hash: {})",
                                &line[..RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES],
                                hash.to_hex()
                            )
                        } else {
                            format!("  | {line}")
                        }
                    })
                    .collect()
            }
        }
    }
}

fn push_hash_with_optional_resolution(
    lines: &mut Vec<String>,
    label: &str,
    hash: &akmon_journal::Hash,
    store: &akmon_journal::RedbObjectStore,
    resolve: bool,
    verbose: bool,
    binary: BinaryMode,
) {
    let rendered = if verbose {
        format_hash_full(hash)
    } else {
        truncate_hash(hash)
    };
    lines.push(format!("  {label}: {rendered}"));
    if resolve {
        lines.extend(format_resolved_human(
            hash,
            resolve_object(store, hash),
            binary,
        ));
    }
}

fn attempt_status_name(status: &akmon_journal::AttemptStatus) -> String {
    match status {
        akmon_journal::AttemptStatus::Success => "Success".to_owned(),
        akmon_journal::AttemptStatus::RateLimited => "RateLimited".to_owned(),
        akmon_journal::AttemptStatus::NetworkError => "NetworkError".to_owned(),
        akmon_journal::AttemptStatus::ServerError => "ServerError".to_owned(),
        akmon_journal::AttemptStatus::ClientError => "ClientError".to_owned(),
        akmon_journal::AttemptStatus::Cancelled => "Cancelled".to_owned(),
        akmon_journal::AttemptStatus::Other(other) => format!("Other({other})"),
    }
}

fn summarize_attempts(attempts: &[akmon_journal::AttemptRecord]) -> String {
    if attempts.is_empty() {
        return "0 attempts".to_owned();
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for attempt in attempts {
        let name = attempt_status_name(&attempt.status);
        if let Some((_, count)) = counts.iter_mut().find(|(status, _)| *status == name) {
            *count += 1;
        } else {
            counts.push((name, 1));
        }
    }
    let mut parts = Vec::with_capacity(counts.len());
    for (status, count) in counts {
        if count == 1 {
            parts.push(format!("1 {status}"));
        } else {
            parts.push(format!("{count} {status}"));
        }
    }
    let noun = if attempts.len() == 1 {
        "1 attempt"
    } else {
        "attempts"
    };
    if attempts.len() == 1 {
        format!("{noun}: {}", parts.join(", "))
    } else {
        format!("{} {noun}: {}", attempts.len(), parts.join(", "))
    }
}

fn event_kind_name(kind: &akmon_journal::EventKind) -> &'static str {
    match kind {
        akmon_journal::EventKind::SessionStart { .. } => "SessionStart",
        akmon_journal::EventKind::UserTurn { .. } => "UserTurn",
        akmon_journal::EventKind::ProviderCall { .. } => "ProviderCall",
        akmon_journal::EventKind::ToolCall { .. } => "ToolCall",
        akmon_journal::EventKind::RetrievalCall { .. } => "RetrievalCall",
        akmon_journal::EventKind::PermissionGate { .. } => "PermissionGate",
        akmon_journal::EventKind::AssistantTurn { .. } => "AssistantTurn",
        akmon_journal::EventKind::SessionEnd { .. } => "SessionEnd",
    }
}

fn format_iso_utc(epoch_seconds: i64, nanos: u32) -> String {
    DateTime::<Utc>::from_timestamp(epoch_seconds, nanos)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "invalid-timestamp".to_owned())
}

fn format_time_utc(epoch_seconds: i64, nanos: u32) -> String {
    DateTime::<Utc>::from_timestamp(epoch_seconds, nanos)
        .map(|dt| dt.format("%H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| "invalid-time".to_owned())
}

fn format_parents_verbose(parents: &[akmon_journal::Hash]) -> Vec<String> {
    if parents.is_empty() {
        return vec!["  parent: none".to_owned()];
    }
    if parents.len() == 1 {
        return vec![format!("  parent: {}", format_hash_full(&parents[0]))];
    }
    let mut lines = vec!["  parents:".to_owned()];
    for parent in parents {
        lines.push(format!("    - {}", format_hash_full(parent)));
    }
    lines
}

fn format_event_summary(
    seq: usize,
    hash: &akmon_journal::Hash,
    event: &akmon_journal::Event,
    store: &akmon_journal::RedbObjectStore,
    resolve: bool,
    binary: BinaryMode,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "[{seq}] {}  hash={}",
        event_kind_name(&event.kind),
        truncate_hash(hash)
    ));
    match &event.kind {
        akmon_journal::EventKind::SessionStart {
            cwd_hash,
            config_hash,
        } => {
            push_hash_with_optional_resolution(
                &mut lines, "cwd_hash", cwd_hash, store, resolve, false, binary,
            );
            push_hash_with_optional_resolution(
                &mut lines,
                "config_hash",
                config_hash,
                store,
                resolve,
                false,
                binary,
            );
        }
        akmon_journal::EventKind::UserTurn { prompt_hash } => {
            push_hash_with_optional_resolution(
                &mut lines,
                "prompt_hash",
                prompt_hash,
                store,
                resolve,
                false,
                binary,
            );
        }
        akmon_journal::EventKind::ProviderCall {
            provider_id,
            attempts,
            stream_hash,
        } => {
            lines.push(format!("  provider: {provider_id}"));
            lines.push(format!("  attempts: {}", summarize_attempts(attempts)));
            if let Some(stream_hash) = stream_hash.as_ref() {
                push_hash_with_optional_resolution(
                    &mut lines,
                    "stream_hash",
                    stream_hash,
                    store,
                    resolve,
                    false,
                    binary,
                );
            } else {
                lines.push("  stream_hash: none".to_owned());
            }
        }
        akmon_journal::EventKind::ToolCall {
            tool_id,
            input_hash,
            output_hash,
            side_effects_hash,
        } => {
            lines.push(format!("  tool: {tool_id}"));
            push_hash_with_optional_resolution(
                &mut lines,
                "input_hash",
                input_hash,
                store,
                resolve,
                false,
                binary,
            );
            push_hash_with_optional_resolution(
                &mut lines,
                "output_hash",
                output_hash,
                store,
                resolve,
                false,
                binary,
            );
            lines.push(format!(
                "  side_effects: {}",
                if side_effects_hash.is_some() {
                    "yes"
                } else {
                    "no"
                }
            ));
            if let Some(side_effects_hash) = side_effects_hash.as_ref()
                && resolve
            {
                lines.extend(format_resolved_human(
                    side_effects_hash,
                    resolve_object(store, side_effects_hash),
                    binary,
                ));
            }
        }
        akmon_journal::EventKind::RetrievalCall {
            index_id,
            query_hash,
            results_hash,
        } => {
            lines.push(format!("  index_id: {index_id}"));
            push_hash_with_optional_resolution(
                &mut lines,
                "query_hash",
                query_hash,
                store,
                resolve,
                false,
                binary,
            );
            push_hash_with_optional_resolution(
                &mut lines,
                "results_hash",
                results_hash,
                store,
                resolve,
                false,
                binary,
            );
        }
        akmon_journal::EventKind::PermissionGate {
            policy_id,
            decision,
            context_hash,
        } => {
            lines.push(format!("  policy: {policy_id}"));
            lines.push(format!("  decision: {decision}"));
            push_hash_with_optional_resolution(
                &mut lines,
                "context_hash",
                context_hash,
                store,
                resolve,
                false,
                binary,
            );
        }
        akmon_journal::EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        } => {
            push_hash_with_optional_resolution(
                &mut lines,
                "message_hash",
                message_hash,
                store,
                resolve,
                false,
                binary,
            );
            lines.push(format!(
                "  tool_calls: {}",
                if tool_calls_hash.is_some() {
                    "yes"
                } else {
                    "no"
                }
            ));
            if let Some(tool_calls_hash) = tool_calls_hash.as_ref()
                && resolve
            {
                lines.extend(format_resolved_human(
                    tool_calls_hash,
                    resolve_object(store, tool_calls_hash),
                    binary,
                ));
            }
        }
        akmon_journal::EventKind::SessionEnd { summary_hash } => {
            if let Some(summary_hash) = summary_hash.as_ref() {
                push_hash_with_optional_resolution(
                    &mut lines,
                    "summary_hash",
                    summary_hash,
                    store,
                    resolve,
                    false,
                    binary,
                );
            } else {
                lines.push("  summary_hash: none".to_owned());
            }
        }
    }
    lines.join("\n")
}

fn format_event_verbose(
    seq: usize,
    hash: &akmon_journal::Hash,
    event: &akmon_journal::Event,
    store: &akmon_journal::RedbObjectStore,
    resolve: bool,
    binary: BinaryMode,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{}  seq={seq}  hash={}",
        event_kind_name(&event.kind),
        format_hash_full(hash)
    ));
    lines.extend(format_parents_verbose(&event.parents));
    lines.push(format!(
        "  emitted_at: {}",
        format_iso_utc(
            event.emitted_at.unix_timestamp(),
            event.emitted_at.nanosecond()
        )
    ));
    match &event.kind {
        akmon_journal::EventKind::SessionStart {
            cwd_hash,
            config_hash,
        } => {
            push_hash_with_optional_resolution(
                &mut lines, "cwd_hash", cwd_hash, store, resolve, true, binary,
            );
            push_hash_with_optional_resolution(
                &mut lines,
                "config_hash",
                config_hash,
                store,
                resolve,
                true,
                binary,
            );
        }
        akmon_journal::EventKind::UserTurn { prompt_hash } => {
            push_hash_with_optional_resolution(
                &mut lines,
                "prompt_hash",
                prompt_hash,
                store,
                resolve,
                true,
                binary,
            );
        }
        akmon_journal::EventKind::ProviderCall {
            provider_id,
            attempts,
            stream_hash,
        } => {
            lines.push(format!("  provider: {provider_id}"));
            lines.push("  attempts:".to_owned());
            for attempt in attempts {
                lines.push(format!(
                    "    [{}] {}  started={}  ended={}",
                    attempt.attempt_number,
                    attempt_status_name(&attempt.status),
                    format_time_utc(
                        attempt.started_at.unix_timestamp(),
                        attempt.started_at.nanosecond()
                    ),
                    format_time_utc(
                        attempt.ended_at.unix_timestamp(),
                        attempt.ended_at.nanosecond()
                    )
                ));
                lines.push(format!(
                    "        request_hash: {}",
                    format_hash_full(&attempt.request_hash)
                ));
                if resolve {
                    lines.extend(format_resolved_human(
                        &attempt.request_hash,
                        resolve_object(store, &attempt.request_hash),
                        binary,
                    ));
                }
                lines.push(format!(
                    "        response_hash: {}",
                    format_optional_hash_full(attempt.response_hash.as_ref())
                ));
                if let Some(response_hash) = attempt.response_hash.as_ref()
                    && resolve
                {
                    lines.extend(format_resolved_human(
                        response_hash,
                        resolve_object(store, response_hash),
                        binary,
                    ));
                }
                lines.push(format!(
                    "        stream_hash: {}",
                    format_optional_hash_full(attempt.stream_hash.as_ref())
                ));
                if let Some(stream_hash) = attempt.stream_hash.as_ref()
                    && resolve
                {
                    lines.extend(format_resolved_human(
                        stream_hash,
                        resolve_object(store, stream_hash),
                        binary,
                    ));
                }
                lines.push(format!(
                    "        error: {}",
                    attempt
                        .error_message
                        .as_ref()
                        .map_or_else(|| "none".to_owned(), std::clone::Clone::clone)
                ));
            }
            lines.push(format!(
                "  stream_hash: {}",
                format_optional_hash_full(stream_hash.as_ref())
            ));
            if let Some(stream_hash) = stream_hash.as_ref()
                && resolve
            {
                lines.extend(format_resolved_human(
                    stream_hash,
                    resolve_object(store, stream_hash),
                    binary,
                ));
            }
        }
        akmon_journal::EventKind::ToolCall {
            tool_id,
            input_hash,
            output_hash,
            side_effects_hash,
        } => {
            lines.push(format!("  tool: {tool_id}"));
            push_hash_with_optional_resolution(
                &mut lines,
                "input_hash",
                input_hash,
                store,
                resolve,
                true,
                binary,
            );
            push_hash_with_optional_resolution(
                &mut lines,
                "output_hash",
                output_hash,
                store,
                resolve,
                true,
                binary,
            );
            lines.push(format!(
                "  side_effects_hash: {}",
                format_optional_hash_full(side_effects_hash.as_ref())
            ));
            if let Some(side_effects_hash) = side_effects_hash.as_ref()
                && resolve
            {
                lines.extend(format_resolved_human(
                    side_effects_hash,
                    resolve_object(store, side_effects_hash),
                    binary,
                ));
            }
        }
        akmon_journal::EventKind::RetrievalCall {
            index_id,
            query_hash,
            results_hash,
        } => {
            lines.push(format!("  index_id: {index_id}"));
            push_hash_with_optional_resolution(
                &mut lines,
                "query_hash",
                query_hash,
                store,
                resolve,
                true,
                binary,
            );
            push_hash_with_optional_resolution(
                &mut lines,
                "results_hash",
                results_hash,
                store,
                resolve,
                true,
                binary,
            );
        }
        akmon_journal::EventKind::PermissionGate {
            policy_id,
            decision,
            context_hash,
        } => {
            lines.push(format!("  policy: {policy_id}"));
            lines.push(format!("  decision: {decision}"));
            push_hash_with_optional_resolution(
                &mut lines,
                "context_hash",
                context_hash,
                store,
                resolve,
                true,
                binary,
            );
        }
        akmon_journal::EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        } => {
            push_hash_with_optional_resolution(
                &mut lines,
                "message_hash",
                message_hash,
                store,
                resolve,
                true,
                binary,
            );
            lines.push(format!(
                "  tool_calls_hash: {}",
                format_optional_hash_full(tool_calls_hash.as_ref())
            ));
            if let Some(tool_calls_hash) = tool_calls_hash.as_ref()
                && resolve
            {
                lines.extend(format_resolved_human(
                    tool_calls_hash,
                    resolve_object(store, tool_calls_hash),
                    binary,
                ));
            }
        }
        akmon_journal::EventKind::SessionEnd { summary_hash } => {
            lines.push(format!(
                "  summary_hash: {}",
                format_optional_hash_full(summary_hash.as_ref())
            ));
            if let Some(summary_hash) = summary_hash.as_ref()
                && resolve
            {
                lines.extend(format_resolved_human(
                    summary_hash,
                    resolve_object(store, summary_hash),
                    binary,
                ));
            }
        }
    }
    lines.join("\n")
}

fn format_event(
    seq: usize,
    hash: &akmon_journal::Hash,
    event: &akmon_journal::Event,
    store: &akmon_journal::RedbObjectStore,
    resolve: bool,
    verbose: bool,
    binary: BinaryMode,
) -> String {
    if verbose {
        format_event_verbose(seq, hash, event, store, resolve, binary)
    } else {
        format_event_summary(seq, hash, event, store, resolve, binary)
    }
}

fn run_inspect(
    session_id: uuid::Uuid,
    journal: Option<PathBuf>,
    format: InspectFormat,
    verbose: bool,
    resolve: bool,
    binary: BinaryMode,
) -> ExitCode {
    if matches!(binary, BinaryMode::Hex | BinaryMode::Base64) && !resolve {
        eprintln!("error: --binary {binary:?} requires --resolve");
        return ExitCode::from(2);
    }
    let journal_dir = match journal {
        Some(path) => path,
        None => match default_journal_dir() {
            Ok(path) => path,
            Err(err) => {
                let msg = format!("cannot resolve default journal directory: {err}");
                if matches!(format, InspectFormat::Json) {
                    let _ = print_inspect_json_error(inspect_error_category(&msg), msg);
                } else {
                    eprintln!("akmon: inspect: {msg}");
                }
                return ExitCode::from(3);
            }
        },
    };
    let handle = match open_journal_read_only(journal_dir.as_path(), session_id) {
        Ok(h) => h,
        Err(err) => {
            let msg = format!(
                "cannot open journal {} for session {}: {err}",
                journal_dir.display(),
                session_id
            );
            if matches!(format, InspectFormat::Json) {
                let _ = print_inspect_json_error(inspect_error_category(&msg), msg);
            } else {
                eprintln!("akmon: inspect: {msg}");
            }
            return ExitCode::from(3);
        }
    };
    let graph = handle
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let history = match graph.history() {
        Ok(h) => h,
        Err(err) => {
            let msg = format!("failed to read session history: {err}");
            if matches!(format, InspectFormat::Json) {
                let _ = print_inspect_json_error("history_read_error", msg);
            } else {
                eprintln!("akmon: inspect: {msg}");
            }
            return ExitCode::from(3);
        }
    };
    if matches!(format, InspectFormat::Json) {
        let body = inspect_report_v1(
            session_id,
            journal_dir.as_path(),
            handle.store.as_ref(),
            resolve,
            &history,
        );
        if let Err(err) = print_inspect_json_report(&body) {
            eprintln!("akmon: inspect: failed to render JSON output: {err}");
            return ExitCode::from(3);
        }
        return ExitCode::SUCCESS;
    }
    let journal_display =
        dunce::canonicalize(journal_dir.as_path()).unwrap_or_else(|_| journal_dir.clone());
    println!("session: {session_id}");
    println!("events: {}", history.len());
    println!("journal: {}", journal_display.display());
    for (idx, (hash, event)) in history.iter().enumerate() {
        println!();
        println!(
            "{}",
            format_event(
                idx,
                hash,
                event,
                handle.store.as_ref(),
                resolve,
                verbose,
                binary
            )
        );
    }
    ExitCode::SUCCESS
}

#[cfg(feature = "semantic-index")]
type SemanticIndexParts = (
    Arc<RwLock<Option<akmon_index::RepoIndex>>>,
    Arc<std::sync::Mutex<TextEmbedding>>,
);

/// Builds the tool registry; [`ShellTool`] is registered only when at least one `--shell-allow` pattern is set
/// and `plan_mode` is `false`.
///
/// [`WebFetchTool`] is registered only when `web_fetch` is true (`--web-fetch`).
///
/// When `plan_mode` is `true`, only read-oriented tools are registered (no write, shell, git, MCP added here).
///
/// [`SemanticSearchTool`] is included when built with `semantic-index` and `semantic` is [`Some`].
fn build_tool_registry(
    shell_allow: &[String],
    web_fetch: bool,
    has_git_root: bool,
    plan_mode: bool,
    #[cfg(feature = "semantic-index")] semantic: Option<SemanticIndexParts>,
) -> Vec<Box<dyn akmon_tools::Tool>> {
    if plan_mode {
        let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
            Box::new(ReadFileTool::new()),
            Box::new(ListDirectoryTool::new()),
            Box::new(SearchTool::new()),
            Box::new(AskFollowupTool),
            Box::new(TodoWriteTool),
            Box::new(MemoryWriteTool),
        ];
        if web_fetch {
            tools.push(Box::new(WebFetchTool::new()));
        }
        #[cfg(feature = "semantic-index")]
        if let Some((slot, emb)) = semantic {
            tools.push(Box::new(SemanticSearchTool::new(slot, Some(emb))));
        }
        return tools;
    }
    let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
        Box::new(ReadFileTool::new()),
        Box::new(WriteFileTool::new()),
        Box::new(ListDirectoryTool::new()),
        Box::new(SearchTool::new()),
        Box::new(EditTool::new()),
        Box::new(PatchTool::new()),
        Box::new(ApplyPatchTool::new()),
        Box::new(AskFollowupTool),
        Box::new(TodoWriteTool),
        Box::new(MemoryWriteTool),
    ];
    if web_fetch {
        tools.push(Box::new(WebFetchTool::new()));
    }
    if !shell_allow.is_empty() {
        tools.push(Box::new(ShellTool::new(shell_allow.to_vec())));
    }
    #[cfg(feature = "semantic-index")]
    if let Some((slot, emb)) = semantic {
        tools.push(Box::new(SemanticSearchTool::new(slot, Some(emb))));
    }
    if has_git_root {
        tools.push(Box::new(GitTool::new()));
    }
    tools
}

#[cfg(feature = "semantic-index")]
#[allow(clippy::too_many_arguments)]
fn cli_attach_specs_subagent(
    tools: &mut Vec<Box<dyn akmon_tools::Tool>>,
    cli: &Cli,
    has_git_root: bool,
    plan_mode: bool,
    provider: &Arc<dyn LlmProvider>,
    sandbox: &Arc<Sandbox>,
    akmon_md: &Option<String>,
    semantic_parts: Option<SemanticIndexParts>,
) {
    tools.push(Box::new(ReadSpecTool::new()));
    if !plan_mode {
        tools.push(Box::new(WriteSpecTool::new()));
    }
    let shell_allow = cli.shell_allow.clone();
    let web_fetch = cli.web_fetch;
    let semantic = semantic_parts.clone();
    let plan_for_sub = plan_mode;
    let factory: SubagentToolFactory = Arc::new(move || {
        build_tool_registry(
            &shell_allow,
            web_fetch,
            has_git_root,
            plan_for_sub,
            semantic.clone(),
        )
    });
    let rt = Arc::new(SubagentRuntime {
        provider: Arc::clone(provider),
        sandbox: Arc::clone(sandbox),
        akmon_md: akmon_md.clone(),
        plan_mode,
        confirmation_timeout_secs: 30,
        tool_factory: factory,
    });
    tools.push(Box::new(SpawnSubagentTool::new(rt)));
}

#[cfg(not(feature = "semantic-index"))]
fn cli_attach_specs_subagent(
    tools: &mut Vec<Box<dyn akmon_tools::Tool>>,
    cli: &Cli,
    has_git_root: bool,
    plan_mode: bool,
    provider: &Arc<dyn LlmProvider>,
    sandbox: &Arc<Sandbox>,
    akmon_md: &Option<String>,
) {
    tools.push(Box::new(ReadSpecTool::new()));
    if !plan_mode {
        tools.push(Box::new(WriteSpecTool::new()));
    }
    let shell_allow = cli.shell_allow.clone();
    let web_fetch = cli.web_fetch;
    let plan_for_sub = plan_mode;
    let factory: SubagentToolFactory =
        Arc::new(move || build_tool_registry(&shell_allow, web_fetch, has_git_root, plan_for_sub));
    let rt = Arc::new(SubagentRuntime {
        provider: Arc::clone(provider),
        sandbox: Arc::clone(sandbox),
        akmon_md: akmon_md.clone(),
        plan_mode,
        confirmation_timeout_secs: 30,
        tool_factory: factory,
    });
    tools.push(Box::new(SpawnSubagentTool::new(rt)));
}

fn load_user_global_config() -> AkmonGlobalConfig {
    akmon_config::akmon_config_path()
        .as_ref()
        .and_then(|p| akmon_config::load_config_from(p).ok())
        .unwrap_or_default()
}

fn provider_resolution_for_cli(cli: &Cli) -> ProviderResolutionTrace {
    let global = load_user_global_config();
    llm_connect_from_cli(cli, &global, cli.model.clone()).explain_provider_resolution()
}

fn coalesce_opt(a: Option<String>, b: Option<String>) -> Option<String> {
    a.filter(|s| !s.trim().is_empty())
        .or_else(|| b.filter(|s| !s.trim().is_empty()))
}

/// Builds [`LlmConnectConfig`] from CLI flags merged with `~/.akmon/config.toml`.
pub(crate) fn llm_connect_from_cli(
    cli: &Cli,
    global: &AkmonGlobalConfig,
    model: String,
) -> LlmConnectConfig {
    let azure_ver = if cli.azure_api_version.is_empty() {
        global
            .azure_api_version
            .clone()
            .unwrap_or_else(|| "2024-02-01".into())
    } else {
        cli.azure_api_version.clone()
    };
    LlmConnectConfig {
        model,
        ollama_url: cli.ollama_url.clone(),
        anthropic_api_key: coalesce_opt(
            cli.anthropic_key.clone(),
            global.anthropic_api_key.clone(),
        ),
        openrouter_api_key: coalesce_opt(
            cli.openrouter_key.clone(),
            global.openrouter_api_key.clone(),
        ),
        openai_api_key: coalesce_opt(cli.openai_key.clone(), global.openai_api_key.clone()),
        groq_api_key: coalesce_opt(cli.groq_key.clone(), global.groq_api_key.clone()),
        azure_openai_endpoint: coalesce_opt(
            cli.azure_endpoint.clone(),
            global.azure_openai_endpoint.clone(),
        ),
        azure_openai_api_key: coalesce_opt(
            cli.azure_key.clone(),
            global.azure_openai_api_key.clone(),
        ),
        azure_api_version: azure_ver,
        bedrock_explicit: cli.bedrock,
        aws_region: cli.aws_region.clone(),
        openai_compatible_url: coalesce_opt(
            cli.openai_compatible_url.clone(),
            global.openai_compatible_url.clone(),
        ),
        openai_compatible_api_key: coalesce_opt(
            cli.openai_compatible_key.clone(),
            global.openai_compatible_api_key.clone(),
        ),
    }
}

fn resolve_llm(
    cli: &Cli,
    global: &AkmonGlobalConfig,
    model: String,
) -> Result<Arc<dyn LlmProvider>, ProviderError> {
    llm_connect_from_cli(cli, global, model).resolve()
}

/// Resolves the planner model for architect mode: `--planner-model`, then `~/.akmon/config.toml` `[architect]`, then `llama3.2`.
pub(crate) fn planner_model_for_tui(cli: &Cli) -> String {
    let global = load_user_global_config();
    cli.planner_model
        .clone()
        .or(global.architect.planner_model.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "llama3.2".into())
}

/// Default JSONL audit path under `project_root`: `.akmon/audit/{session_id}.jsonl`.
fn default_audit_log_path(project_root: &Path, session_id: uuid::Uuid) -> PathBuf {
    project_root
        .join(".akmon")
        .join("audit")
        .join(format!("{session_id}.jsonl"))
}

/// Resolves the audit file path: explicit `--audit-log` or the default under `.akmon/audit/`.
fn resolve_audit_log_path(
    project_root: &Path,
    session_id: uuid::Uuid,
    custom: Option<PathBuf>,
) -> PathBuf {
    match custom {
        Some(p) => p,
        None => default_audit_log_path(project_root, session_id),
    }
}

/// Default evidence path under `project_root`: `.akmon/evidence/{session_id}.json`.
fn default_evidence_path(project_root: &Path, session_id: uuid::Uuid) -> PathBuf {
    project_root
        .join(".akmon")
        .join("evidence")
        .join(format!("{session_id}.json"))
}

/// Resolves evidence output path: explicit `--evidence-path` or default under `.akmon/evidence/`.
fn resolve_evidence_path(
    project_root: &Path,
    session_id: uuid::Uuid,
    custom: Option<PathBuf>,
) -> PathBuf {
    match custom {
        Some(p) => p,
        None => default_evidence_path(project_root, session_id),
    }
}

fn build_evidence_artifact<S, G>(
    session: &AgentSession<S, G>,
    audit_log_path: &Path,
) -> EvidenceArtifact
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    let snapshot = session.evidence_data();
    let reliability = snapshot.reliability_metrics.clone();
    let replay = snapshot
        .replay_metadata
        .unwrap_or_else(|| replay_metadata_for_report(session));
    let (audit_chain_valid, session_final_hash, note) = match verify_audit_jsonl(audit_log_path) {
        Ok(s) => (true, s.session_final_hash, None),
        Err(e) => (
            false,
            None,
            Some(format!("audit chain validation failed: {e}")),
        ),
    };
    let total = snapshot.tools.len() as u64;
    let success = snapshot.tools.iter().filter(|t| t.success).count() as u64;
    let failure = total.saturating_sub(success);
    let mut artifact = EvidenceArtifact::new(
        snapshot.session_id,
        Utc::now(),
        replay,
        EvidenceAudit {
            audit_log_path: audit_log_path.to_string_lossy().into_owned(),
            audit_chain_valid,
            session_final_hash,
        },
        EvidencePolicy {
            allow: snapshot.policy.allow,
            deny: snapshot.policy.deny,
            prompted: snapshot.policy.prompted,
            decision_samples: snapshot.policy.decision_samples,
        },
        EvidenceTools {
            timeline: snapshot
                .tools
                .into_iter()
                .map(|t| EvidenceToolCall {
                    name: t.name,
                    success: t.success,
                    message: t.message,
                })
                .collect(),
            total,
            success,
            failure,
        },
        snapshot.files_touched,
        EvidenceVerification {
            outcomes: Vec::new(),
            unavailable_reason: Some("verification commands not collected in this run".into()),
        },
    );
    artifact.reliability_metrics = reliability;
    if let Some(n) = note {
        artifact.notes.push(n);
    }
    artifact
}

fn write_evidence_artifact<S, G>(
    session: &AgentSession<S, G>,
    audit_log_path: &Path,
    evidence_path: &Path,
) -> std::io::Result<()>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    let artifact = build_evidence_artifact(session, audit_log_path);
    write_evidence_json(evidence_path, &artifact)
}

fn resolve_resume_session_id(cli: &Cli, project_root: &Path) -> Result<Option<uuid::Uuid>, String> {
    if cli.continue_last {
        let index = session_index::SessionIndex::load();
        let Some(entry) = index.get_for_project(project_root) else {
            return Err("No previous session found for this directory.".into());
        };
        let u = uuid::Uuid::parse_str(&entry.session_id)
            .map_err(|e| format!("invalid session id in index: {e}"))?;
        return Ok(Some(u));
    }
    if let Some(ref s) = cli.resume_session {
        return session_transcript::resolve_session_id_from_cli_arg(s).map(Some);
    }
    Ok(None)
}

fn sandbox_for_cli(
    project_root: PathBuf,
    has_git_root: bool,
    add_dirs: &[PathBuf],
) -> Arc<Sandbox> {
    let extra: Vec<PathBuf> = add_dirs
        .iter()
        .filter_map(|p| dunce::canonicalize(p).ok())
        .collect();
    if extra.is_empty() {
        Arc::new(Sandbox::with_git_root(project_root, has_git_root))
    } else {
        Arc::new(Sandbox::with_additional_roots_git(
            project_root,
            extra,
            has_git_root,
        ))
    }
}

fn model_messages_to_tui(msgs: Vec<Message>) -> Vec<akmon_tui::TuiMessage> {
    use akmon_tui::TuiMessage;
    msgs.into_iter()
        .filter_map(|m| match m.role {
            MessageRole::User => Some(TuiMessage::User { content: m.content }),
            MessageRole::Assistant => Some(TuiMessage::Assistant {
                content: m.content,
                complete: true,
            }),
            _ => None,
        })
        .collect()
}

fn exit_reason_ok<S, G>(session: &AgentSession<S, G>) -> ExitReason
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    match session.last_run_exit() {
        SessionRunExit::BudgetLimit => ExitReason::BudgetLimit,
        SessionRunExit::Completed => ExitReason::Completed,
    }
}

fn exit_reason_err(e: &AgentError) -> ExitReason {
    match e {
        AgentError::IterationLimitReached { .. } => ExitReason::MaxTurns,
        _ => ExitReason::Error,
    }
}

fn headless_persist<S, G>(
    project_root: &Path,
    session: &AgentSession<S, G>,
    model: &str,
    started_at: chrono::DateTime<chrono::Utc>,
) where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    let msgs: Vec<Message> = session.context_messages().to_vec();
    let started_str = started_at.to_rfc3339();
    if let Err(e) = session_transcript::save_headless_session_file(
        session_transcript::HeadlessSessionSnapshot {
            session_id: session.session_id(),
            project_root,
            model,
            messages: &msgs,
            started_at_rfc3339: &started_str,
            total_input_tokens: session.total_input_tokens(),
            total_cache_read_tokens: session.total_cache_read_tokens(),
            total_output_tokens: session.total_output_tokens(),
        },
    ) {
        eprintln!("akmon: warning: could not save session snapshot: {e}");
    }
    let mut index = session_index::SessionIndex::load();
    index.record(
        project_root,
        session_index::SessionEntry {
            session_id: session.session_id().to_string(),
            model: model.into(),
            started_at: started_str,
            turn_count: session.user_turns_finished,
        },
    );
}

/// Prints [`AgentEvent`]s for the TTY and forwards interactive policy replies.
async fn run_event_printer(
    mut ev_rx: mpsc::Receiver<AgentEvent>,
    policy_tx: mpsc::Sender<InteractivePolicyReply>,
    output: OutputFormat,
) {
    while let Some(ev) = ev_rx.recv().await {
        match ev {
            AgentEvent::TextDelta { text } => {
                if output == OutputFormat::Text {
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                }
            }
            AgentEvent::ToolCallDispatched { name, .. } => {
                if output == OutputFormat::Text {
                    println!("\n→ {name}");
                }
            }
            AgentEvent::ToolCallCompleted {
                name,
                success,
                message,
                ..
            } => {
                if output == OutputFormat::Text {
                    if success {
                        println!("✓ {name}");
                    } else {
                        eprintln!("✗ {name}: {message}");
                    }
                }
            }
            AgentEvent::StatusInfo { message } => {
                if output == OutputFormat::Text {
                    eprintln!("{message}");
                }
            }
            AgentEvent::SummarizationStarted => {
                if output == OutputFormat::Text {
                    eprintln!("akmon: context summarization started…");
                }
            }
            AgentEvent::ContextSummarized {
                messages_replaced,
                tokens_freed,
            } => {
                if output == OutputFormat::Text {
                    eprintln!(
                        "akmon: context summarized (messages_replaced={messages_replaced}, tokens_freed≈{tokens_freed})"
                    );
                }
            }
            AgentEvent::ConfirmationRequired {
                description,
                diff_preview,
            } => {
                eprintln!("{description}");
                if let Some(diff) = diff_preview {
                    let colored = akmon_tools::colorize_unified_diff(&diff);
                    eprint!("{colored}");
                }
                let line: String = tokio::task::spawn_blocking(|| {
                    print!("Allow? [y=once / Y=remember session / n=N]: ");
                    let _ = std::io::stdout().flush();
                    let mut buf = String::new();
                    let _ = std::io::stdin().read_line(&mut buf);
                    buf
                })
                .await
                .unwrap_or_default();
                let t = line.trim();
                let reply = if t == "Y" {
                    InteractivePolicyReply {
                        verdict: PolicyVerdict::Allow,
                        remember_for_session: true,
                        allow_all_writes_session: false,
                        shell_allow_prefix: None,
                    }
                } else if t.eq_ignore_ascii_case("y") {
                    InteractivePolicyReply::allow_once()
                } else {
                    InteractivePolicyReply::deny()
                };
                let _ = policy_tx.send(reply).await;
            }
            AgentEvent::UsageReport {
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                ..
            } => {
                if output == OutputFormat::Text
                    && (cache_read_tokens > 0 || cache_creation_tokens > 0)
                {
                    eprintln!(
                        "akmon: tokens — input:{input_tokens} cache_hit:{cache_read_tokens} cache_write:{cache_creation_tokens}"
                    );
                }
            }
            AgentEvent::Done => {
                if output == OutputFormat::Text {
                    println!("\nDone.");
                }
            }
            AgentEvent::Error { error, .. } => {
                if output == OutputFormat::Text {
                    eprintln!("{error}");
                }
            }
            _ => {}
        }
    }
}

fn seed_project_dot_akmon_if_applicable(project_root: &Path, has_git_root: bool) {
    if !cli_project::should_ensure_project_dot_akmon(project_root, has_git_root) {
        return;
    }
    if let Err(e) = akmon_core::ensure_dot_akmon_layout(project_root) {
        eprintln!("akmon: warning: could not create .akmon directories: {e}");
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            exit_early_config_error(&cli, format!("cannot read current directory: {e}"), None, 2);
        }
    };

    let (project_root, has_git_root) = cli_project::resolve_sandbox_root(&cwd);
    if !has_git_root {
        eprintln!("akmon: no git repository found.");
        eprintln!(
            "Using cwd as sandbox: {}",
            dunce::canonicalize(&project_root)
                .unwrap_or_else(|_| project_root.clone())
                .display()
        );
        eprintln!("Run git init to enable git features.");
    }

    match &cli.command {
        Some(Commands::Init) => {
            return cli_project::run_init(&cli, &project_root).await;
        }
        Some(Commands::Import(args)) => {
            let provider = match cli_project::resolve_provider(&cli) {
                Ok(p) => p,
                Err(e) => {
                    exit_early_config_error(&cli, e.to_string(), None, 2);
                }
            };
            return match import_cmd::run_import(args.clone(), project_root.clone(), provider).await
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("akmon: import: {e:#}");
                    ExitCode::from(1)
                }
            };
        }
        Some(Commands::Export(args)) => {
            return match export_cmd::run_export(args.clone(), &project_root) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("akmon: export: {e:#}");
                    ExitCode::from(1)
                }
            };
        }
        Some(Commands::Bundle(bundle_args)) => match &bundle_args.command {
            BundleCommands::Export(args) => {
                return run_bundle_export(
                    args.session_id,
                    args.output.clone(),
                    args.journal.clone(),
                    args.format,
                );
            }
        },
        Some(Commands::New(args)) => {
            return cli_project::run_new(&cli, args, &cwd).await;
        }
        Some(Commands::Config(c)) => {
            return config_cmd::run_config(c.clone(), &cli).await;
        }
        Some(Commands::Doctor(d)) => {
            let global = load_user_global_config();
            let connect = llm_connect_from_cli(&cli, &global, cli.model.clone());
            return doctor_cmd::run_doctor(d.clone(), cli.output == OutputFormat::Json, &connect)
                .await;
        }
        Some(Commands::Audit(a)) => {
            return audit_cmd::run_audit(a.clone(), cli.output == OutputFormat::Json);
        }
        Some(Commands::Evidence(e)) => {
            return evidence_cmd::run_evidence(e.clone(), cli.output == OutputFormat::Json);
        }
        Some(Commands::Slo(s)) => {
            let global = load_user_global_config();
            return slo_cmd::run_slo(s.clone(), cli.output == OutputFormat::Json, &global);
        }
        Some(Commands::Policy(p)) => {
            let global = load_user_global_config();
            return policy_cmd::run_policy(
                p.clone(),
                cli.output == OutputFormat::Json,
                &project_root,
                &global,
            );
        }
        Some(Commands::Scout(s)) => {
            return scout_cmd::run_scout(
                s.clone(),
                &project_root,
                cli.output == OutputFormat::Json,
                cli.max_budget_usd,
            );
        }
        Some(Commands::Spec(sc)) => {
            return spec_cmd::run_spec(&cli, &project_root, sc.clone()).await;
        }
        Some(Commands::Verify {
            session_id,
            journal,
            format,
            verbose,
        }) => {
            return run_verify(*session_id, journal.clone(), *format, *verbose);
        }
        Some(Commands::Inspect {
            session_id,
            journal,
            format,
            verbose,
            resolve,
            binary,
        }) => {
            return run_inspect(
                *session_id,
                journal.clone(),
                *format,
                *verbose,
                *resolve,
                *binary,
            );
        }
        Some(Commands::Chat) | None => {}
    }

    let Some(task) = cli.task.clone() else {
        seed_project_dot_akmon_if_applicable(&project_root, has_git_root);
        let global = load_user_global_config();
        let azure_ver = if cli.azure_api_version.is_empty() {
            global
                .azure_api_version
                .clone()
                .unwrap_or_else(|| "2024-02-01".into())
        } else {
            cli.azure_api_version.clone()
        };
        let akmon_content = match load_akmon_md(&project_root) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("akmon: warning: failed to read AKMON.md: {e}");
                None
            }
        };
        let dossier_block = match &cli.dossier {
            Some(path) => match load_dossier_system_block(path) {
                Ok(block) => Some(block),
                Err(e) => {
                    eprintln!("akmon: invalid dossier {}: {e}", path.display());
                    return ExitCode::from(2);
                }
            },
            None => None,
        };
        let akmon_content = merge_akmon_with_dossier(akmon_content, dossier_block);
        let has_akmon_md = project_root.join("AKMON.md").is_file();

        let mode_label = if cli.yes { "AUTO" } else { "INTERACTIVE" };
        let resolved_resume = match resolve_resume_session_id(&cli, &project_root) {
            Ok(x) => x,
            Err(e) => {
                exit_resume_session_error(&cli, e);
            }
        };
        let session_id = resolved_resume.unwrap_or_else(uuid::Uuid::new_v4);
        let resume_messages = if resolved_resume.is_some() {
            match session_transcript::load_resume_messages(session_id, &project_root) {
                Ok(m) => Some(model_messages_to_tui(m)),
                Err(e) => {
                    eprintln!("akmon: warning: could not load session transcript: {e}");
                    None
                }
            }
        } else {
            None
        };
        let audit_log_path =
            resolve_audit_log_path(&project_root, session_id, cli.audit_log.clone());

        #[cfg(feature = "semantic-index")]
        let mut index_thread: Option<std::thread::JoinHandle<()>> = None;
        #[cfg(not(feature = "semantic-index"))]
        let index_thread: Option<std::thread::JoinHandle<()>> = None;

        #[cfg(feature = "semantic-index")]
        let semantic_index: Option<akmon_tui::SemanticIndexSlot> = if cli.index {
            let sandbox = sandbox_for_cli(project_root.clone(), has_git_root, &cli.add_dirs);
            let index_path = project_root.join(".akmon").join("index.bin");
            if !index_path.is_file() {
                eprintln!("akmon: downloading embedding model (~22MB) on first use...");
            }
            match TextEmbedding::try_new(
                TextInitOptions::default().with_show_download_progress(true),
            ) {
                Ok(m) => {
                    let emb = Arc::new(std::sync::Mutex::new(m));
                    let slot = Arc::new(RwLock::new(None));

                    if index_path.is_file() {
                        match akmon_index::load_index(&index_path) {
                            Ok(idx) => {
                                eprintln!(
                                    "akmon: semantic index loaded ({} files, {} chunks)",
                                    idx.file_count, idx.chunk_count
                                );
                                *slot.write().await = Some(idx);
                            }
                            Err(e) => {
                                eprintln!(
                                    "akmon: warning: could not load semantic index, rebuilding: {e}"
                                );
                                let slot_bg = Arc::clone(&slot);
                                let root_bg = project_root.clone();
                                let sandbox_bg = Arc::clone(&sandbox);
                                let emb_bg = Arc::clone(&emb);
                                let path_bg = index_path.clone();
                                index_thread = spawn_semantic_index_os_thread(
                                    root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                                );
                                poll_index_ready_up_to_3s(&slot).await;
                            }
                        }
                    } else {
                        let slot_bg = Arc::clone(&slot);
                        let root_bg = project_root.clone();
                        let sandbox_bg = Arc::clone(&sandbox);
                        let emb_bg = Arc::clone(&emb);
                        let path_bg = index_path.clone();
                        index_thread = spawn_semantic_index_os_thread(
                            root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                        );
                        poll_index_ready_up_to_3s(&slot).await;
                    }

                    Some((slot, emb))
                }
                Err(e) => {
                    eprintln!("akmon: warning: semantic index disabled (embedding model): {e}");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(not(feature = "semantic-index"))]
        let semantic_index = {
            if cli.index {
                eprintln!(
                    "akmon: --index is ignored (this binary was built without the `semantic-index` feature)."
                );
            }
            None
        };

        let tui_config = TuiLaunchConfig {
            version: env!("CARGO_PKG_VERSION").to_string(),
            project_root: project_root.clone(),
            model_name: cli.model.clone(),
            mode_label: mode_label.to_string(),
            session_id,
            max_iterations: AgentConfig::default().max_iterations,
            index_enabled: cli.index,
            anthropic_key: coalesce_opt(
                cli.anthropic_key.clone(),
                global.anthropic_api_key.clone(),
            ),
            openrouter_key: coalesce_opt(
                cli.openrouter_key.clone(),
                global.openrouter_api_key.clone(),
            ),
            openai_key: coalesce_opt(cli.openai_key.clone(), global.openai_api_key.clone()),
            groq_key: coalesce_opt(cli.groq_key.clone(), global.groq_api_key.clone()),
            azure_endpoint: coalesce_opt(
                cli.azure_endpoint.clone(),
                global.azure_openai_endpoint.clone(),
            ),
            azure_key: coalesce_opt(cli.azure_key.clone(), global.azure_openai_api_key.clone()),
            azure_api_version: azure_ver,
            bedrock: cli.bedrock,
            aws_region: cli.aws_region.clone(),
            openai_compatible_url: coalesce_opt(
                cli.openai_compatible_url.clone(),
                global.openai_compatible_url.clone(),
            ),
            openai_compatible_key: coalesce_opt(
                cli.openai_compatible_key.clone(),
                global.openai_compatible_api_key.clone(),
            ),
            ollama_url: cli.ollama_url.clone(),
            shell_allow: cli.shell_allow.clone(),
            web_fetch: cli.web_fetch,
            yes_web: cli.yes_web,
            auto_yes: cli.yes,
            mcp_servers: cli.mcp_server.clone(),
            audit_log_path,
            akmon_md: akmon_content,
            has_akmon_md,
            sandbox_has_git_root: has_git_root,
            semantic_index,
            auto_commit: cli.auto_commit,
            planner_model: planner_model_for_tui(&cli),
            display_theme: global.display.theme,
            session_display_name: cli.session_name.clone(),
            resume_messages,
            model_estimates: global.model_estimates.clone(),
        };
        let tui_outcome = akmon_tui::run_interactive(tui_config).await;
        if let Some(handle) = index_thread {
            eprintln!("akmon: waiting for index to finish building...");
            let _ = handle.join();
        }
        return match tui_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("akmon: TUI error: {e}");
                ExitCode::from(1)
            }
        };
    };

    seed_project_dot_akmon_if_applicable(&project_root, has_git_root);

    let akmon_content = match load_akmon_md(&project_root) {
        Ok(c) => c,
        Err(e) => {
            exit_early_config_error(&cli, format!("failed to read AKMON.md: {e}"), None, 2);
        }
    };
    let dossier_block = match &cli.dossier {
        Some(path) => match load_dossier_system_block(path) {
            Ok(block) => Some(block),
            Err(e) => {
                exit_early_config_error(
                    &cli,
                    format!("invalid dossier {}: {e}", path.display()),
                    None,
                    2,
                );
            }
        },
        None => None,
    };
    let akmon_content = merge_akmon_with_dossier(akmon_content, dossier_block);

    if cli.plan && cli.architect {
        exit_early_config_error(
            &cli,
            "--plan cannot be combined with --architect".into(),
            None,
            2,
        );
    }

    if cli.output == OutputFormat::Text {
        eprintln!("akmon: project root: {}", project_root.display());
        match &akmon_content {
            Some(text) => eprintln!("akmon: AKMON.md loaded ({} bytes)", text.len()),
            None => eprintln!("akmon: AKMON.md not found (optional)"),
        }
    }

    let resolved_resume = match resolve_resume_session_id(&cli, &project_root) {
        Ok(x) => x,
        Err(e) => {
            exit_resume_session_error(&cli, e);
        }
    };
    let session_id = resolved_resume.unwrap_or_else(uuid::Uuid::new_v4);
    let headless_started_at = chrono::Utc::now();
    let resume_ctx: Vec<Message> = if resolved_resume.is_some() {
        session_transcript::load_resume_messages(session_id, &project_root).unwrap_or_else(|e| {
            eprintln!("akmon: warning: could not load session transcript: {e}");
            Vec::new()
        })
    } else {
        Vec::new()
    };

    let audit_log_path = resolve_audit_log_path(&project_root, session_id, cli.audit_log.clone());
    let evidence_path = resolve_evidence_path(&project_root, session_id, cli.evidence_path.clone());
    let global = load_user_global_config();
    let agent_config = AgentConfig {
        session_id,
        auto_commit: cli.auto_commit,
        max_budget_usd: cli.max_budget_usd,
        fallback_model: cli.fallback_model.clone(),
        model_estimates: global.model_estimates.clone(),
        ..Default::default()
    };

    let resolved_policy = policy_cmd::resolve_effective_policy(
        &project_root,
        &global,
        &policy_cmd::PolicyResolutionOptions {
            profile: cli.policy_profile.map(Into::into),
            pack_paths: cli.policy_pack.clone(),
            override_path: cli.policy_override.clone(),
        },
    );
    let policy_mode = match resolved_policy {
        Ok(Some(resolved)) => PolicyEngineMode::Configured(resolved.effective),
        Ok(None) => {
            if cli.yes {
                if cli.web_fetch && cli.yes_web {
                    PolicyEngineMode::AutoApproveReadsAndFetch {
                        confirm_writes: true,
                    }
                } else {
                    PolicyEngineMode::AutoApproveReads {
                        confirm_writes: true,
                    }
                }
            } else {
                PolicyEngineMode::Interactive
            }
        }
        Err(e) => {
            exit_early_config_error(
                &cli,
                format!("failed to resolve effective policy: {e}"),
                None,
                2,
            );
        }
    };
    let policy = Arc::new(PolicyEngine::new(policy_mode));
    let sandbox = sandbox_for_cli(project_root.clone(), has_git_root, &cli.add_dirs);

    #[cfg(feature = "semantic-index")]
    let mut index_thread: Option<std::thread::JoinHandle<()>> = None;
    #[cfg(not(feature = "semantic-index"))]
    let mut index_thread: Option<std::thread::JoinHandle<()>> = None;

    #[cfg(feature = "semantic-index")]
    let semantic_parts: Option<SemanticIndexParts> = if cli.index {
        let index_path = project_root.join(".akmon").join("index.bin");
        if !index_path.is_file() {
            eprintln!("akmon: downloading embedding model (~22MB) on first use...");
        }
        match TextEmbedding::try_new(TextInitOptions::default().with_show_download_progress(true)) {
            Ok(m) => {
                let emb = Arc::new(std::sync::Mutex::new(m));
                let slot = Arc::new(RwLock::new(None));

                if index_path.is_file() {
                    match akmon_index::load_index(&index_path) {
                        Ok(idx) => {
                            eprintln!(
                                "akmon: semantic index loaded ({} files, {} chunks)",
                                idx.file_count, idx.chunk_count
                            );
                            *slot.write().await = Some(idx);
                        }
                        Err(e) => {
                            eprintln!(
                                "akmon: warning: could not load semantic index, rebuilding: {e}"
                            );
                            let slot_bg = Arc::clone(&slot);
                            let root_bg = project_root.clone();
                            let sandbox_bg = Arc::clone(&sandbox);
                            let emb_bg = Arc::clone(&emb);
                            let path_bg = index_path.clone();
                            index_thread = spawn_semantic_index_os_thread(
                                root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                            );
                            poll_index_ready_up_to_3s(&slot).await;
                        }
                    }
                } else {
                    let slot_bg = Arc::clone(&slot);
                    let root_bg = project_root.clone();
                    let sandbox_bg = Arc::clone(&sandbox);
                    let emb_bg = Arc::clone(&emb);
                    let path_bg = index_path.clone();
                    index_thread = spawn_semantic_index_os_thread(
                        root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                    );
                    poll_index_ready_up_to_3s(&slot).await;
                }

                Some((slot, emb))
            }
            Err(e) => {
                eprintln!("akmon: warning: semantic index disabled (embedding model): {e}");
                None
            }
        }
    } else {
        None
    };

    #[cfg(not(feature = "semantic-index"))]
    if cli.index {
        eprintln!(
            "akmon: --index is ignored (this binary was built without the `semantic-index` feature)."
        );
    }

    if cli.plan {
        let provider = match resolve_llm(&cli, &global, cli.model.clone()) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
            }
        };
        let mut tools = build_tool_registry(
            &cli.shell_allow,
            cli.web_fetch,
            has_git_root,
            true,
            #[cfg(feature = "semantic-index")]
            semantic_parts.clone(),
        );
        #[cfg(feature = "semantic-index")]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            true,
            &provider,
            &sandbox,
            &akmon_content,
            semantic_parts.clone(),
        );
        #[cfg(not(feature = "semantic-index"))]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            true,
            &provider,
            &sandbox,
            &akmon_content,
        );
        let plan_agent_config = AgentConfig {
            auto_commit: false,
            ..agent_config.clone()
        };
        let journal = match open_default_journal_handle(plan_agent_config.session_id) {
            Ok(j) => j,
            Err(e) => {
                exit_early_config_error(&cli, format!("journal: {e}"), Some(&mut index_thread), 2)
            }
        };
        let mut session = match AgentSession::new(
            plan_agent_config,
            Arc::clone(&policy),
            provider,
            tools,
            Arc::clone(&sandbox),
            akmon_content.clone(),
            true,
            journal,
        ) {
            Ok(s) => s,
            Err(e) => {
                exit_early_config_error(&cli, format!("session: {e}"), Some(&mut index_thread), 2)
            }
        };
        if !resume_ctx.is_empty() {
            session.restore_context_from_messages(resume_ctx.clone());
        }
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
        let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));
        let mut policy_opt = Some(policy_rx);
        let run_outcome = session
            .run(task.clone(), ev_tx, &mut policy_opt, &mut None, None)
            .await;
        drop(policy_opt);
        let _ = printer.await;
        let plan_body = session.result_text().to_string();
        let saved_path = match akmon_core::save_plan_markdown(&project_root, &task, &plan_body) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(
                    &cli,
                    format!("failed to save plan: {e}"),
                    Some(&mut index_thread),
                    2,
                );
            }
        };
        if cli.output == OutputFormat::Text {
            println!("{plan_body}");
            println!();
            println!("Plan saved to {}", saved_path.display());
            println!();
            println!("Review:  cat {}", saved_path.display());
            println!("Edit:    $EDITOR {}", saved_path.display());
            println!(
                "Implement: akmon --task 'implement the plan in {}'",
                saved_path.display()
            );
        }
        if let Err(e) = write_audit_jsonl(&audit_log_path, session.audit_events()) {
            eprintln!(
                "akmon: failed to write audit log {}: {e}",
                audit_log_path.display()
            );
        }
        if run_outcome.is_ok()
            && let Err(e) = write_evidence_artifact(&session, &audit_log_path, &evidence_path)
        {
            eprintln!(
                "akmon: warning: failed to write evidence artifact {}: {e}",
                evidence_path.display()
            );
        }
        let _ = write_handoff_file(&session, &project_root, &cli.model);
        headless_persist(&project_root, &session, &cli.model, headless_started_at);
        if let Some(handle) = index_thread {
            eprintln!("akmon: waiting for index to finish building...");
            eprintln!(
                "akmon: (CPU-bound embedding — more `akmon:` lines may appear until the index is saved)"
            );
            let _ = handle.join();
        }
        if cli.output == OutputFormat::Json {
            let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
                Ok(()) => ("success", None),
                Err(e) => ("error", Some(e.to_string())),
            };
            let exit_reason = match &run_outcome {
                Ok(()) => exit_reason_ok(&session),
                Err(e) => exit_reason_err(e),
            };
            let report = RunReport {
                session_id: session.session_id().to_string(),
                status,
                exit_reason,
                result: plan_body,
                tool_calls: session.tool_call_summaries().to_vec(),
                error: error_opt,
                audit_log_path: audit_log_path.to_string_lossy().into_owned(),
                usage: RunUsageSummary {
                    total_input_tokens: session.total_input_tokens(),
                    total_cache_read_tokens: session.total_cache_read_tokens(),
                    total_output_tokens: session.total_output_tokens(),
                },
                cost_usd: session.total_cost_usd(),
                cache_read_tokens: u64::from(session.total_cache_read_tokens()),
                files_written: session
                    .modified_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                replay_metadata: replay_metadata_for_report(&session),
                reliability_metrics: session.reliability_metrics(),
                provider_resolution: Some(provider_resolution_for_cli(&cli)),
            };
            let json_line = match serde_json::to_string(&report) {
                Ok(s) => s,
                Err(e) => {
                    print_json_early_error_and_exit(format!("failed to serialize run report: {e}"));
                }
            };
            println!("{json_line}");
        }
        return match run_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                if cli.output == OutputFormat::Text {
                    eprintln!("akmon: {e}");
                }
                exit_code_for_agent_error(&e)
            }
        };
    }

    if cli.architect {
        let planner_model = planner_model_for_tui(&cli);
        let provider_planner = match resolve_llm(&cli, &global, planner_model.clone()) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
            }
        };
        let mut tools_planner = build_tool_registry(
            &cli.shell_allow,
            cli.web_fetch,
            has_git_root,
            true,
            #[cfg(feature = "semantic-index")]
            semantic_parts.clone(),
        );
        #[cfg(feature = "semantic-index")]
        cli_attach_specs_subagent(
            &mut tools_planner,
            &cli,
            has_git_root,
            true,
            &provider_planner,
            &sandbox,
            &akmon_content,
            semantic_parts.clone(),
        );
        #[cfg(not(feature = "semantic-index"))]
        cli_attach_specs_subagent(
            &mut tools_planner,
            &cli,
            has_git_root,
            true,
            &provider_planner,
            &sandbox,
            &akmon_content,
        );
        let planner_agent_config = AgentConfig {
            auto_commit: false,
            ..agent_config.clone()
        };
        let journal = match open_default_journal_handle(planner_agent_config.session_id) {
            Ok(j) => j,
            Err(e) => {
                exit_early_config_error(&cli, format!("journal: {e}"), Some(&mut index_thread), 2)
            }
        };
        let mut planner_session = match AgentSession::new(
            planner_agent_config,
            Arc::clone(&policy),
            provider_planner,
            tools_planner,
            Arc::clone(&sandbox),
            akmon_content.clone(),
            true,
            journal,
        ) {
            Ok(s) => s,
            Err(e) => {
                exit_early_config_error(&cli, format!("session: {e}"), Some(&mut index_thread), 2)
            }
        };
        if !resume_ctx.is_empty() {
            planner_session.restore_context_from_messages(resume_ctx.clone());
        }
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
        let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));
        let mut policy_opt = Some(policy_rx);
        let plan_run = planner_session
            .run(task.clone(), ev_tx, &mut policy_opt, &mut None, None)
            .await;
        drop(policy_opt);
        let _ = printer.await;
        if let Err(e) = plan_run {
            if let Err(audit_err) =
                write_audit_jsonl(&audit_log_path, planner_session.audit_events())
            {
                eprintln!(
                    "akmon: failed to write audit log {}: {audit_err}",
                    audit_log_path.display()
                );
            }
            if let Some(handle) = index_thread {
                eprintln!("akmon: waiting for index to finish building...");
                let _ = handle.join();
            }
            if cli.output == OutputFormat::Text {
                eprintln!("akmon: {e}");
            }
            return exit_code_for_agent_error(&e);
        }
        let plan_text = planner_session.result_text().to_string();
        eprintln!("akmon: architect — plan complete (planner: {planner_model})");
        if let Err(e) = akmon_core::save_plan_markdown(&project_root, &task, &plan_text) {
            eprintln!("akmon: warning: failed to save plan file: {e}");
        }
        let provider_main = match resolve_llm(&cli, &global, cli.model.clone()) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
            }
        };
        let mut tools = build_tool_registry(
            &cli.shell_allow,
            cli.web_fetch,
            has_git_root,
            false,
            #[cfg(feature = "semantic-index")]
            semantic_parts.clone(),
        );
        for url in &cli.mcp_server {
            let server = McpServerConfig {
                name: url.clone(),
                url: url.clone(),
                description: String::new(),
            };
            match discover_mcp_tools(&server).await {
                Ok(mcp_tools) => {
                    eprintln!(
                        "akmon: MCP server {} — {} tools registered",
                        url,
                        mcp_tools.len()
                    );
                    for t in mcp_tools {
                        tools.push(Box::new(t));
                    }
                }
                Err(e) => {
                    eprintln!("akmon: MCP server {url} unavailable: {e}");
                }
            }
        }
        #[cfg(feature = "semantic-index")]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            false,
            &provider_main,
            &sandbox,
            &akmon_content,
            semantic_parts.clone(),
        );
        #[cfg(not(feature = "semantic-index"))]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            false,
            &provider_main,
            &sandbox,
            &akmon_content,
        );
        let journal = match open_default_journal_handle(agent_config.session_id) {
            Ok(j) => j,
            Err(e) => {
                exit_early_config_error(&cli, format!("journal: {e}"), Some(&mut index_thread), 2)
            }
        };
        let mut session = match AgentSession::new(
            agent_config,
            Arc::clone(&policy),
            provider_main,
            tools,
            Arc::clone(&sandbox),
            akmon_content,
            false,
            journal,
        ) {
            Ok(s) => s,
            Err(e) => {
                exit_early_config_error(&cli, format!("session: {e}"), Some(&mut index_thread), 2)
            }
        };
        let impl_task = format!(
            "Implement this plan exactly:\n\n{plan_text}\n\nOriginal task: {task}\n\nFollow the plan step by step.\nDo not deviate from the plan without explaining why."
        );
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
        let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));
        let mut policy_opt = Some(policy_rx);
        let run_outcome = session
            .run(impl_task, ev_tx, &mut policy_opt, &mut None, None)
            .await;
        drop(policy_opt);
        let _ = printer.await;
        let mut combined_audit: Vec<AuditEvent> = Vec::new();
        combined_audit.extend(planner_session.audit_events().iter().cloned());
        combined_audit.extend(session.audit_events().iter().cloned());
        if let Err(e) = write_audit_jsonl(&audit_log_path, &combined_audit) {
            eprintln!(
                "akmon: failed to write audit log {}: {e}",
                audit_log_path.display()
            );
        }
        if run_outcome.is_ok()
            && let Err(e) = write_evidence_artifact(&session, &audit_log_path, &evidence_path)
        {
            eprintln!(
                "akmon: warning: failed to write evidence artifact {}: {e}",
                evidence_path.display()
            );
        }
        let _ = write_handoff_file(&session, &project_root, &cli.model);
        headless_persist(&project_root, &session, &cli.model, headless_started_at);
        if let Some(handle) = index_thread {
            eprintln!("akmon: waiting for index to finish building...");
            eprintln!(
                "akmon: (CPU-bound embedding — more `akmon:` lines may appear until the index is saved)"
            );
            let _ = handle.join();
        }
        if cli.output == OutputFormat::Json {
            let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
                Ok(()) => ("success", None),
                Err(e) => ("error", Some(e.to_string())),
            };
            let exit_reason = match &run_outcome {
                Ok(()) => exit_reason_ok(&session),
                Err(e) => exit_reason_err(e),
            };
            let report = RunReport {
                session_id: session.session_id().to_string(),
                status,
                exit_reason,
                result: session.result_text().to_string(),
                tool_calls: session.tool_call_summaries().to_vec(),
                error: error_opt,
                audit_log_path: audit_log_path.to_string_lossy().into_owned(),
                usage: RunUsageSummary {
                    total_input_tokens: session.total_input_tokens(),
                    total_cache_read_tokens: session.total_cache_read_tokens(),
                    total_output_tokens: session.total_output_tokens(),
                },
                cost_usd: session.total_cost_usd(),
                cache_read_tokens: u64::from(session.total_cache_read_tokens()),
                files_written: session
                    .modified_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                replay_metadata: replay_metadata_for_report(&session),
                reliability_metrics: session.reliability_metrics(),
                provider_resolution: Some(provider_resolution_for_cli(&cli)),
            };
            let json_line = match serde_json::to_string(&report) {
                Ok(s) => s,
                Err(e) => {
                    print_json_early_error_and_exit(format!("failed to serialize run report: {e}"));
                }
            };
            println!("{json_line}");
        }
        return match run_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                if cli.output == OutputFormat::Text {
                    eprintln!("akmon: {e}");
                }
                exit_code_for_agent_error(&e)
            }
        };
    }

    let provider = match resolve_llm(&cli, &global, cli.model.clone()) {
        Ok(p) => p,
        Err(e) => {
            exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
        }
    };

    let mut tools = build_tool_registry(
        &cli.shell_allow,
        cli.web_fetch,
        has_git_root,
        false,
        #[cfg(feature = "semantic-index")]
        semantic_parts.clone(),
    );
    for url in &cli.mcp_server {
        let server = McpServerConfig {
            name: url.clone(),
            url: url.clone(),
            description: String::new(),
        };
        match discover_mcp_tools(&server).await {
            Ok(mcp_tools) => {
                eprintln!(
                    "akmon: MCP server {} — {} tools registered",
                    url,
                    mcp_tools.len()
                );
                for t in mcp_tools {
                    tools.push(Box::new(t));
                }
            }
            Err(e) => {
                eprintln!("akmon: MCP server {url} unavailable: {e}");
            }
        }
    }
    #[cfg(feature = "semantic-index")]
    cli_attach_specs_subagent(
        &mut tools,
        &cli,
        has_git_root,
        false,
        &provider,
        &sandbox,
        &akmon_content,
        semantic_parts,
    );
    #[cfg(not(feature = "semantic-index"))]
    cli_attach_specs_subagent(
        &mut tools,
        &cli,
        has_git_root,
        false,
        &provider,
        &sandbox,
        &akmon_content,
    );

    let journal = match open_default_journal_handle(agent_config.session_id) {
        Ok(j) => j,
        Err(e) => {
            exit_early_config_error(&cli, format!("journal: {e}"), Some(&mut index_thread), 2)
        }
    };
    let mut session = match AgentSession::new(
        agent_config,
        Arc::clone(&policy),
        provider,
        tools,
        Arc::clone(&sandbox),
        akmon_content,
        false,
        journal,
    ) {
        Ok(s) => s,
        Err(e) => {
            exit_early_config_error(&cli, format!("session: {e}"), Some(&mut index_thread), 2)
        }
    };

    if !resume_ctx.is_empty() {
        session.restore_context_from_messages(resume_ctx);
    }

    let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
    let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
    let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));

    let mut policy_opt = Some(policy_rx);
    let run_outcome = session
        .run(task, ev_tx, &mut policy_opt, &mut None, None)
        .await;

    drop(policy_opt);

    let _ = printer.await;

    if let Err(e) = write_audit_jsonl(&audit_log_path, session.audit_events()) {
        eprintln!(
            "akmon: failed to write audit log {}: {e}",
            audit_log_path.display()
        );
    }
    if run_outcome.is_ok()
        && let Err(e) = write_evidence_artifact(&session, &audit_log_path, &evidence_path)
    {
        eprintln!(
            "akmon: warning: failed to write evidence artifact {}: {e}",
            evidence_path.display()
        );
    }

    let _ = write_handoff_file(&session, &project_root, &cli.model);

    headless_persist(&project_root, &session, &cli.model, headless_started_at);

    if let Some(handle) = index_thread {
        eprintln!("akmon: waiting for index to finish building...");
        eprintln!(
            "akmon: (CPU-bound embedding — more `akmon:` lines may appear until the index is saved)"
        );
        let _ = handle.join();
    }

    let audit_log_path_str = audit_log_path.to_string_lossy().into_owned();

    if cli.output == OutputFormat::Json {
        let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
            Ok(()) => ("success", None),
            Err(e) => ("error", Some(e.to_string())),
        };
        let exit_reason = match &run_outcome {
            Ok(()) => exit_reason_ok(&session),
            Err(e) => exit_reason_err(e),
        };
        let report = RunReport {
            session_id: session.session_id().to_string(),
            status,
            exit_reason,
            result: session.result_text().to_string(),
            tool_calls: session.tool_call_summaries().to_vec(),
            error: error_opt,
            audit_log_path: audit_log_path_str,
            usage: RunUsageSummary {
                total_input_tokens: session.total_input_tokens(),
                total_cache_read_tokens: session.total_cache_read_tokens(),
                total_output_tokens: session.total_output_tokens(),
            },
            cost_usd: session.total_cost_usd(),
            cache_read_tokens: u64::from(session.total_cache_read_tokens()),
            files_written: session
                .modified_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            replay_metadata: replay_metadata_for_report(&session),
            reliability_metrics: session.reliability_metrics(),
            provider_resolution: Some(provider_resolution_for_cli(&cli)),
        };
        let json_line = match serde_json::to_string(&report) {
            Ok(s) => s,
            Err(e) => {
                print_json_early_error_and_exit(format!("failed to serialize run report: {e}"));
            }
        };
        println!("{json_line}");
        return match run_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(ref e) => exit_code_for_agent_error(e),
        };
    }

    match run_outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("akmon: {e}");
            exit_code_for_agent_error(&e)
        }
    }
}

fn exit_code_for_agent_error(e: &AgentError) -> ExitCode {
    match e {
        AgentError::PolicyDenied { .. } => ExitCode::from(3),
        AgentError::IterationLimitReached { .. }
        | AgentError::ModelError { .. }
        | AgentError::ToolError { .. }
        | AgentError::ResponseTruncated
        | AgentError::SessionFailed { .. }
        | AgentError::InvalidTransition { .. } => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::scout_cmd::{ScoutCandidateFile, ScoutDossier};
    use akmon_journal::{HashAlgorithm, MemoryObjectStore, MemorySessionGraph};
    use akmon_query::JournalHandle;

    fn test_journal_sid(
        session_id: uuid::Uuid,
    ) -> JournalHandle<MemoryObjectStore, MemorySessionGraph> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
            Arc::clone(&store),
            session_id,
        )));
        JournalHandle::new(store, graph)
    }

    fn reg(
        shell_allow: &[String],
        web_fetch: bool,
        has_git_root: bool,
        plan_mode: bool,
    ) -> Vec<Box<dyn akmon_tools::Tool>> {
        build_tool_registry(
            shell_allow,
            web_fetch,
            has_git_root,
            plan_mode,
            #[cfg(feature = "semantic-index")]
            None,
        )
    }

    #[test]
    fn shell_tool_omitted_without_allow_flags() {
        let t = reg(&[], false, false, false);
        assert!(!t.iter().any(|x| x.name() == "shell"));
    }

    #[test]
    fn shell_tool_registered_when_allow_patterns_present() {
        let t = reg(&["echo *".into()], false, false, false);
        assert!(t.iter().any(|x| x.name() == "shell"));
    }

    #[test]
    fn web_fetch_tool_omitted_without_flag() {
        let t = reg(&[], false, false, false);
        assert!(!t.iter().any(|x| x.name() == "web_fetch"));
    }

    #[test]
    fn web_fetch_tool_registered_when_flag_set() {
        let t = reg(&[], true, false, false);
        assert!(t.iter().any(|x| x.name() == "web_fetch"));
    }

    #[test]
    fn git_tool_registered_when_has_git_root() {
        let t = reg(&[], false, true, false);
        assert!(t.iter().any(|x| x.name() == "git"));
        let t2 = reg(&[], false, false, false);
        assert!(!t2.iter().any(|x| x.name() == "git"));
    }

    #[test]
    fn plan_mode_registry_has_reads_only() {
        let t = reg(&["echo *".into()], true, true, true);
        assert!(t.iter().any(|x| x.name() == "read_file"));
        assert!(t.iter().any(|x| x.name() == "list_directory"));
        assert!(t.iter().any(|x| x.name() == "search"));
        assert!(t.iter().any(|x| x.name() == "web_fetch"));
        assert!(!t.iter().any(|x| x.name() == "write_file"));
        assert!(!t.iter().any(|x| x.name() == "shell"));
        assert!(!t.iter().any(|x| x.name() == "git"));
    }

    #[test]
    fn run_report_json_has_expected_shape() {
        let report = RunReport {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            status: "success",
            exit_reason: ExitReason::Completed,
            result: "hello".into(),
            tool_calls: vec![ToolCallSummary {
                name: "read_file".into(),
                success: true,
                message: "ok".into(),
            }],
            error: None,
            audit_log_path: "/tmp/x.jsonl".into(),
            usage: RunUsageSummary {
                total_input_tokens: 10,
                total_cache_read_tokens: 3,
                total_output_tokens: 7,
            },
            cost_usd: 0.01,
            cache_read_tokens: 3,
            files_written: vec!["src/main.rs".into()],
            replay_metadata: ReplayMetadata {
                hash_algorithm: REPLAY_HASH_ALGORITHM.into(),
                provider_name: "ollama".into(),
                model_id: "llama3.2".into(),
                session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
                policy_hash: "a".repeat(64),
                config_hash: "b".repeat(64),
                tool_registry_hash: "c".repeat(64),
                prompt_assembly_hash: Some("d".repeat(64)),
            },
            reliability_metrics: RunReliabilityMetrics {
                tool_calls_total: 1,
                tool_calls_success: 1,
                tool_calls_failure: 0,
                tool_latency_ms_total: 12,
                tool_latency_ms_avg: 12,
                tool_latency_ms_p95: Some(12),
                policy_denials_total: 0,
                retries_total: 0,
                timeouts_total: 0,
            },
            provider_resolution: None,
        };
        let v = serde_json::to_value(&report).expect("serialize");
        assert_eq!(v["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(v["status"], "success");
        assert_eq!(v["exit_reason"], "completed");
        assert_eq!(v["result"], "hello");
        assert!(v["error"].is_null());
        assert_eq!(v["audit_log_path"], "/tmp/x.jsonl");
        assert_eq!(v["usage"]["total_input_tokens"], 10);
        assert_eq!(v["usage"]["total_cache_read_tokens"], 3);
        assert_eq!(v["usage"]["total_output_tokens"], 7);
        assert_eq!(v["cost_usd"], 0.01);
        assert_eq!(v["cache_read_tokens"], 3);
        let files = v["files_written"].as_array().expect("files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "src/main.rs");
        let tools = v["tool_calls"].as_array().expect("array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["success"], true);
        assert_eq!(tools[0]["message"], "ok");
        assert_eq!(v["replay_metadata"]["hash_algorithm"], "sha256");
        assert_eq!(v["replay_metadata"]["provider_name"], "ollama");
        assert_eq!(v["replay_metadata"]["model_id"], "llama3.2");
        assert_eq!(
            v["replay_metadata"]["policy_hash"]
                .as_str()
                .map(|s| s.len()),
            Some(64)
        );
        assert_eq!(v["reliability_metrics"]["tool_calls_total"], 1);
        assert_eq!(v["reliability_metrics"]["tool_latency_ms_avg"], 12);
        assert!(v.get("provider_resolution").is_none());
    }

    #[test]
    fn run_report_reliability_metrics_is_additive_to_existing_schema() {
        let report = RunReport {
            session_id: "s".into(),
            status: "success",
            exit_reason: ExitReason::Completed,
            result: "ok".into(),
            tool_calls: vec![],
            error: None,
            audit_log_path: "/tmp/audit.jsonl".into(),
            usage: RunUsageSummary {
                total_input_tokens: 1,
                total_cache_read_tokens: 0,
                total_output_tokens: 1,
            },
            cost_usd: 0.0,
            cache_read_tokens: 0,
            files_written: vec!["src/lib.rs".into()],
            replay_metadata: ReplayMetadata {
                hash_algorithm: REPLAY_HASH_ALGORITHM.into(),
                provider_name: "ollama".into(),
                model_id: "llama3.2".into(),
                session_id: "s".into(),
                policy_hash: "a".repeat(64),
                config_hash: "b".repeat(64),
                tool_registry_hash: "c".repeat(64),
                prompt_assembly_hash: None,
            },
            reliability_metrics: RunReliabilityMetrics::default(),
            provider_resolution: None,
        };
        let v = serde_json::to_value(&report).expect("serialize");
        assert!(v.get("session_id").is_some());
        assert!(v.get("tool_calls").is_some());
        assert!(v.get("usage").is_some());
        assert!(v.get("replay_metadata").is_some());
        assert!(v.get("reliability_metrics").is_some());
    }

    #[test]
    fn evidence_path_defaults_under_dot_akmon() {
        let root = std::path::Path::new("/tmp/proj");
        let session_id = uuid::Uuid::nil();
        let p = default_evidence_path(root, session_id);
        assert_eq!(
            p,
            root.join(".akmon/evidence/00000000-0000-0000-0000-000000000000.json")
        );
    }

    #[test]
    fn evidence_path_override_is_used() {
        let root = std::path::Path::new("/tmp/proj");
        let session_id = uuid::Uuid::nil();
        let custom = PathBuf::from("/tmp/custom-evidence.json");
        let p = resolve_evidence_path(root, session_id, Some(custom.clone()));
        assert_eq!(p, custom);
    }

    #[test]
    fn t_verify_subcommand_parses_session_id() {
        let cli = Cli::try_parse_from(["akmon", "verify", "550e8400-e29b-41d4-a716-446655440000"])
            .expect("parse verify");
        match cli.command {
            Some(Commands::Verify {
                session_id,
                journal,
                format,
                verbose,
            }) => {
                assert_eq!(
                    session_id.to_string(),
                    "550e8400-e29b-41d4-a716-446655440000"
                );
                assert!(journal.is_none());
                assert_eq!(format, VerifyFormat::Human);
                assert!(!verbose);
            }
            other => panic!("expected verify command, got {other:?}"),
        }
    }

    #[test]
    fn t_verify_subcommand_parses_optional_flags() {
        let cli = Cli::try_parse_from([
            "akmon",
            "verify",
            "550e8400-e29b-41d4-a716-446655440000",
            "--journal",
            "/tmp/journal.redb",
            "--format",
            "json",
            "--verbose",
        ])
        .expect("parse verify flags");
        match cli.command {
            Some(Commands::Verify {
                session_id,
                journal,
                format,
                verbose,
            }) => {
                assert_eq!(
                    session_id.to_string(),
                    "550e8400-e29b-41d4-a716-446655440000"
                );
                assert_eq!(journal, Some(PathBuf::from("/tmp/journal.redb")));
                assert_eq!(format, VerifyFormat::Json);
                assert!(verbose);
            }
            other => panic!("expected verify command, got {other:?}"),
        }
    }

    #[test]
    fn t_verify_subcommand_rejects_invalid_uuid() {
        let err = Cli::try_parse_from(["akmon", "verify", "not-a-uuid"]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value")
                || rendered.contains("invalid character")
                || rendered.contains("UUID"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_inspect_subcommand_parses_session_id() {
        let cli = Cli::try_parse_from(["akmon", "inspect", "550e8400-e29b-41d4-a716-446655440000"])
            .expect("parse inspect");
        match cli.command {
            Some(Commands::Inspect {
                session_id,
                journal,
                format,
                verbose,
                resolve,
                binary,
            }) => {
                assert_eq!(
                    session_id.to_string(),
                    "550e8400-e29b-41d4-a716-446655440000"
                );
                assert!(journal.is_none());
                assert_eq!(format, InspectFormat::Human);
                assert!(!verbose);
                assert!(!resolve);
                assert_eq!(binary, BinaryMode::Meta);
            }
            other => panic!("expected inspect command, got {other:?}"),
        }
    }

    #[test]
    fn t_inspect_subcommand_parses_optional_flags() {
        let cli = Cli::try_parse_from([
            "akmon",
            "inspect",
            "550e8400-e29b-41d4-a716-446655440000",
            "--journal",
            "/tmp/journal.redb",
            "--format",
            "json",
            "--verbose",
            "--resolve",
            "--binary",
            "hex",
        ])
        .expect("parse inspect flags");
        match cli.command {
            Some(Commands::Inspect {
                session_id,
                journal,
                format,
                verbose,
                resolve,
                binary,
            }) => {
                assert_eq!(
                    session_id.to_string(),
                    "550e8400-e29b-41d4-a716-446655440000"
                );
                assert_eq!(journal, Some(PathBuf::from("/tmp/journal.redb")));
                assert_eq!(format, InspectFormat::Json);
                assert!(verbose);
                assert!(resolve);
                assert_eq!(binary, BinaryMode::Hex);
            }
            other => panic!("expected inspect command, got {other:?}"),
        }
    }

    #[test]
    fn t_inspect_subcommand_rejects_invalid_uuid() {
        let err = Cli::try_parse_from(["akmon", "inspect", "not-a-uuid"]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value")
                || rendered.contains("invalid character")
                || rendered.contains("UUID"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_inspect_subcommand_rejects_invalid_format() {
        let err = Cli::try_parse_from([
            "akmon",
            "inspect",
            "550e8400-e29b-41d4-a716-446655440000",
            "--format",
            "yaml",
        ])
        .expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value") || rendered.contains("possible values"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_inspect_subcommand_rejects_invalid_binary_mode() {
        let err = Cli::try_parse_from([
            "akmon",
            "inspect",
            "550e8400-e29b-41d4-a716-446655440000",
            "--binary",
            "raw",
        ])
        .expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value") || rendered.contains("possible values"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_bundle_export_subcommand_parses_session_id() {
        let cli = Cli::try_parse_from([
            "akmon",
            "bundle",
            "export",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .expect("parse bundle export");
        match cli.command {
            Some(Commands::Bundle(bundle)) => match bundle.command {
                BundleCommands::Export(args) => {
                    assert_eq!(
                        args.session_id.to_string(),
                        "550e8400-e29b-41d4-a716-446655440000"
                    );
                    assert!(args.output.is_none());
                    assert!(args.journal.is_none());
                    assert_eq!(args.format, BundleExportFormat::Human);
                }
            },
            other => panic!("expected bundle export command, got {other:?}"),
        }
    }

    #[test]
    fn t_bundle_export_subcommand_parses_optional_flags() {
        let cli = Cli::try_parse_from([
            "akmon",
            "bundle",
            "export",
            "550e8400-e29b-41d4-a716-446655440000",
            "--output",
            "/tmp/session.akmon",
            "--journal",
            "/tmp/journal.redb",
            "--format",
            "json",
        ])
        .expect("parse bundle export flags");
        match cli.command {
            Some(Commands::Bundle(bundle)) => match bundle.command {
                BundleCommands::Export(args) => {
                    assert_eq!(
                        args.session_id.to_string(),
                        "550e8400-e29b-41d4-a716-446655440000"
                    );
                    assert_eq!(args.output, Some(PathBuf::from("/tmp/session.akmon")));
                    assert_eq!(args.journal, Some(PathBuf::from("/tmp/journal.redb")));
                    assert_eq!(args.format, BundleExportFormat::Json);
                }
            },
            other => panic!("expected bundle export command, got {other:?}"),
        }
    }

    #[test]
    fn t_bundle_export_subcommand_rejects_invalid_uuid() {
        let err = Cli::try_parse_from(["akmon", "bundle", "export", "not-a-uuid"])
            .expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value")
                || rendered.contains("invalid character")
                || rendered.contains("UUID"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_bundle_export_subcommand_rejects_missing_uuid() {
        let err = Cli::try_parse_from(["akmon", "bundle", "export"]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("required arguments were not provided")
                || rendered.contains("<SESSION_ID>")
                || rendered.contains("Usage:"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_bundle_subcommand_without_subcommand_rejected() {
        let err = Cli::try_parse_from(["akmon", "bundle"]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("subcommand")
                || rendered.contains("required")
                || rendered.contains("Usage:"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_bundle_export_format_invalid_value_rejected() {
        let err = Cli::try_parse_from([
            "akmon",
            "bundle",
            "export",
            "550e8400-e29b-41d4-a716-446655440000",
            "--format",
            "yaml",
        ])
        .expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value") || rendered.contains("possible values"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn evidence_writer_uses_default_path() {
        let dir = tempfile::tempdir().expect("tmp");
        let session_id = uuid::Uuid::new_v4();
        let audit_path = dir.path().join("audit.jsonl");
        write_audit_jsonl(
            &audit_path,
            &[AuditEvent::AgentStep {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                description: "step".into(),
            }],
        )
        .expect("write audit");
        let sandbox = Arc::new(Sandbox::new(dir.path()));
        let session = AgentSession::new(
            AgentConfig {
                session_id,
                ..Default::default()
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(akmon_models::OllamaBackend::new(
                "http://localhost:11434",
                "llama3.2",
            )),
            vec![],
            sandbox,
            None,
            false,
            test_journal_sid(session_id),
        )
        .expect("session");
        let default_path = default_evidence_path(dir.path(), session_id);
        write_evidence_artifact(&session, &audit_path, &default_path).expect("write evidence");
        assert!(default_path.is_file());
    }

    #[test]
    fn evidence_writer_uses_override_path() {
        let dir = tempfile::tempdir().expect("tmp");
        let session_id = uuid::Uuid::new_v4();
        let audit_path = dir.path().join("audit.jsonl");
        write_audit_jsonl(
            &audit_path,
            &[AuditEvent::AgentStep {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                description: "step".into(),
            }],
        )
        .expect("write audit");
        let sandbox = Arc::new(Sandbox::new(dir.path()));
        let session = AgentSession::new(
            AgentConfig {
                session_id,
                ..Default::default()
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(akmon_models::OllamaBackend::new(
                "http://localhost:11434",
                "llama3.2",
            )),
            vec![],
            sandbox,
            None,
            false,
            test_journal_sid(session_id),
        )
        .expect("session");
        let custom = dir.path().join("custom").join("evidence.json");
        write_evidence_artifact(&session, &audit_path, &custom).expect("write evidence");
        assert!(custom.is_file());
    }

    #[test]
    fn evidence_writer_includes_reliability_metrics_block() {
        let dir = tempfile::tempdir().expect("tmp");
        let session_id = uuid::Uuid::new_v4();
        let audit_path = dir.path().join("audit.jsonl");
        write_audit_jsonl(
            &audit_path,
            &[AuditEvent::AgentStep {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                description: "step".into(),
            }],
        )
        .expect("write audit");
        let sandbox = Arc::new(Sandbox::new(dir.path()));
        let session = AgentSession::new(
            AgentConfig {
                session_id,
                ..Default::default()
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(akmon_models::OllamaBackend::new(
                "http://localhost:11434",
                "llama3.2",
            )),
            vec![],
            sandbox,
            None,
            false,
            test_journal_sid(session_id),
        )
        .expect("session");
        let evidence_path = dir.path().join("evidence.json");
        write_evidence_artifact(&session, &audit_path, &evidence_path).expect("write evidence");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&evidence_path).expect("read"))
                .expect("json");
        assert!(parsed.get("reliability_metrics").is_some());
        assert_eq!(parsed["reliability_metrics"]["tool_calls_total"], 0);
    }

    #[test]
    fn merge_akmon_with_dossier_appends_context_block() {
        let merged = merge_akmon_with_dossier(
            Some("# AKMON\nrules".into()),
            Some("=== Context Scout Dossier ===\nblock".into()),
        );
        let text = merged.expect("merged");
        assert!(text.contains("# AKMON"));
        assert!(text.contains("Context Scout Dossier"));
    }

    #[test]
    fn load_dossier_system_block_success() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("dossier.json");
        let dossier = ScoutDossier {
            schema_version: "context_scout.v1".into(),
            task: "task".into(),
            project_root: dir.path().display().to_string(),
            scanned_paths: vec!["src".into()],
            key_entrypoints: vec!["Cargo.toml".into()],
            candidate_files: vec![ScoutCandidateFile {
                path: "src/main.rs".into(),
                rationale: "path matches task terms: main".into(),
            }],
            related_tests: vec!["tests/main_test.rs".into()],
            constraints: vec!["read-only".into()],
            unresolved_questions: vec![],
            confidence: "medium".into(),
            files_scanned: 10,
            max_files: 20,
            truncated: false,
            generated_at: None,
        };
        let raw = serde_json::to_string_pretty(&dossier).expect("json");
        std::fs::write(&p, raw).expect("write");
        let block = load_dossier_system_block(&p).expect("load");
        assert!(block.contains("Context Scout Dossier"));
        assert!(block.contains("candidate_files"));
    }

    #[test]
    fn load_dossier_system_block_missing_path_errors() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("missing.json");
        let err = load_dossier_system_block(&p).expect_err("must fail");
        assert!(err.contains("failed to read dossier"));
    }

    #[test]
    fn load_dossier_system_block_malformed_json_errors() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("bad.json");
        std::fs::write(&p, "{bad").expect("write");
        let err = load_dossier_system_block(&p).expect_err("must fail");
        assert!(err.contains("invalid dossier JSON"));
    }
}
