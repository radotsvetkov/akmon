//! `akmon bundle` — export, import, and verify AGEF bundles (Item 4.3).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use akmon_bundle::{
    BundleContents, BundleError, DEFAULT_MAX_EVENT_FRAME_LEN, Manifest, ManifestSignature,
    OtelCaptureInfo, Producer, ReadBundleOptions, SCHEME_ED25519, SIG_STATEMENT_VERSION,
    SessionMetadata, SignatureOutcome, SignatureVerificationReport, WriteBundleOptions, key_id,
    otel_capture_info, parse_public_key_hex, public_key_from_pkcs8, read_bundle, sign_statement,
    signing_statement, verify_bundle, verify_manifest_signatures, write_bundle,
};
use akmon_journal::{
    AGEF_SPEC_VERSION, Event, EventKind, Hash, HashAlgorithm, ObjectStore, RedbObjectStore,
    RedbSessionGraph, SessionGraph, referenced_object_hashes_for_kind,
};
use akmon_query::{default_journal_dir, journal_contains_session, open_journal_read_only};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

/// Required capture level for `akmon bundle verify --require-capture`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RequireCapture {
    /// Require that the session captured full message content.
    Full,
}

/// Output format for `akmon bundle export`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum BundleExportFormat {
    /// Human-readable status messages.
    #[default]
    Human,
    /// Machine-readable JSON status messages.
    Json,
}

/// Output format for `akmon bundle import` status messages.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum BundleImportFormat {
    /// Human-readable status messages.
    #[default]
    Human,
    /// Machine-readable JSON status messages.
    Json,
}

/// Stable JSON shape for `akmon bundle export --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleExportReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// AGEF specification version written into the bundle manifest.
    agef_version: String,
    /// Hyphenated session UUID.
    session_id: String,
    /// Absolute or relative path of the written bundle file.
    output_path: String,
    /// Number of session events exported.
    events_exported: u64,
    /// Number of distinct content-addressed objects exported.
    objects_exported: u64,
    /// On-disk bundle size in bytes after write.
    bundle_size_bytes: u64,
}

/// JSON shape emitted when bundle export cannot complete.
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleExportError {
    /// CLI crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
}

/// Machine-readable bundle verification result (`akmon bundle verify` / `import --verify-only`).
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleVerifyReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// AGEF specification version declared by the bundle manifest.
    agef_version: String,
    /// Path to the bundle file that was verified.
    bundle_path: String,
    /// Session UUID from the bundle manifest.
    session_id: String,
    /// Number of events decoded from `events.bin`.
    events_in_bundle: u64,
    /// Number of objects decoded from `objects/`.
    objects_in_bundle: u64,
    /// Holistic verdict: integrity clean, no invalid signatures, and (with `--require-signature`)
    /// at least one signature verified.
    passed: bool,
    /// Collected integrity violations (empty when structurally clean).
    violations: Vec<BundleViolation>,
    /// Per-signature verification results (empty when the bundle is unsigned).
    #[serde(default)]
    signatures: Vec<CliSignatureReport>,
    /// OTEL-import capture level, or `null` for native (non-OTEL) bundles. A native bundle is
    /// full-fidelity by construction; an OTEL import with `level == "structural"` carries metadata
    /// only (the source telemetry did not capture message content).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capture: Option<CliCaptureField>,
}

/// OTEL-import capture metadata for `akmon bundle verify --format json` (F1).
#[derive(Debug, Serialize, Deserialize)]
pub struct CliCaptureField {
    /// Capture level: `full` (message content captured) or `structural` (metadata only).
    level: String,
    /// Source OpenTelemetry semantic-conventions version (for example `1.37.0`).
    source_semconv: String,
}

/// One signature-verification result for `akmon bundle verify --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CliSignatureReport {
    /// `key_id` from the manifest entry (hex SHA-256 of the signer public key).
    key_id: String,
    /// Signature scheme (`ed25519`).
    scheme: String,
    /// Outcome: `verified`, `invalid`, `unverified_no_key`, `unsupported_scheme`, or `malformed`.
    outcome: String,
}

/// Stable lowercase outcome string for [`CliSignatureReport::outcome`].
fn signature_outcome_str(outcome: &SignatureOutcome) -> &'static str {
    match outcome {
        SignatureOutcome::Verified => "verified",
        SignatureOutcome::Invalid => "invalid",
        SignatureOutcome::UnverifiedNoKey => "unverified_no_key",
        SignatureOutcome::UnsupportedScheme => "unsupported_scheme",
        SignatureOutcome::Malformed => "malformed",
    }
}

/// Maps the library signature report into CLI-compatible JSON entries.
fn cli_signature_reports(report: &SignatureVerificationReport) -> Vec<CliSignatureReport> {
    report
        .checks
        .iter()
        .map(|c| CliSignatureReport {
            key_id: c.key_id.clone(),
            scheme: c.scheme.clone(),
            outcome: signature_outcome_str(&c.outcome).to_owned(),
        })
        .collect()
}

/// Machine-readable bundle import result (`akmon bundle import --format json`).
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleImportReportV1 {
    /// CLI crate version that produced this report.
    akmon_version: String,
    /// AGEF specification version declared by the bundle manifest.
    agef_version: String,
    /// Path to the bundle file that was imported.
    bundle_path: String,
    /// Session UUID from the bundle manifest before any rename.
    original_session_id: String,
    /// Session UUID written into the target journal (rename target or original).
    imported_session_id: String,
    /// Number of events imported from `events.bin`.
    events_imported: u64,
    /// Number of objects decoded from `objects/`.
    objects_total: u64,
    /// Number of objects newly written into the local store.
    objects_new: u64,
    /// Number of objects already present in the local store with matching bytes.
    objects_existing: u64,
    /// Resolved journal directory where import was written.
    journal_path: String,
}

/// One bundle verification violation for JSON output.
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleViolation {
    /// Stable category identifier.
    category: String,
    /// Event content hash in hex when applicable.
    event_hash: Option<String>,
    /// Object hash in hex when applicable.
    object_hash: Option<String>,
    /// Human-readable explanation.
    message: String,
}

/// JSON shape emitted when `akmon bundle import` cannot complete (I/O, manifest, collision, or verification failure).
#[derive(Debug, Serialize)]
pub(crate) struct BundleImportInfraError {
    /// CLI crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
    /// Target session UUID when `category` is `session_id_collision`.
    #[serde(skip_serializing_if = "Option::is_none")]
    colliding_session_id: Option<String>,
}
/// Arguments for `akmon bundle`.
#[derive(Args, Debug, Clone)]
pub struct BundleArgs {
    /// Bundle command to execute.
    #[command(subcommand)]
    pub(crate) command: BundleCommands,
}

/// Nested bundle subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum BundleCommands {
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
  2 — usage error (e.g., output path already exists)\n\
  3 — journal/session not found, incomplete store, malformed session bounds, or bundle write error")]
    Export(BundleExportArgs),
    /// Import an AGEF bundle into the local journal.
    #[command(long_about = "Import an AGEF bundle into the local journal.\n\n\
Reads the named .akmon bundle file, validates it per AGEF v0.1.1 (manifest schema, framing, \
hash-chain integrity, object closure, head consistency), and writes its objects and events into \
the target journal as a new session.\n\n\
Use `akmon bundle verify` (or `--verify-only` here) to validate without modifying the journal. \
Use --rename-to <NEW_UUID> to import a bundle whose session_id already exists locally, assigning a \
different ID. Use --allow-extra-files to accept bundles that include files outside the AGEF \
normative set (default behavior is strict reject).\n\n\
Examples:\n\
  akmon bundle import audit.akmon\n\
  akmon bundle verify audit.akmon\n\
  akmon bundle import audit.akmon --rename-to 7c9a...\n\
  akmon bundle import audit.akmon --format json\n\n\
Exit codes:\n\
  0 — bundle imported successfully (or verified if --verify-only)\n\
  1 — verification failed (chain integrity, object closure, head, etc.)\n\
  2 — usage error (e.g., session_id collision without --rename-to)\n\
  3 — I/O or environment error (bundle/journal not found, malformed archive, etc.)")]
    Import(BundleImportArgs),
    /// Verify an AGEF bundle without modifying the local journal.
    #[command(
        long_about = "Verify an AGEF bundle without modifying the local journal.\n\n\
Validates manifest schema, event framing, object re-hashing, hash-chain integrity, object \
closure, and manifest head consistency via the same path as `akmon bundle import \
--verify-only` and the standalone `agef-verify` tool.\n\n\
Examples:\n\
  akmon bundle verify audit.akmon\n\
  akmon bundle verify audit.akmon --format json\n\n\
Exit codes:\n\
  0 — bundle passed all integrity checks\n\
  1 — verification failed (chain integrity, object closure, head, etc.)\n\
  3 — I/O or environment error (bundle not found, malformed archive, etc.)"
    )]
    Verify(BundleVerifyArgs),
    /// Sign an AGEF bundle's session head with an Ed25519 key (AGEF v0.1.2).
    #[command(
        long_about = "Sign an AGEF bundle's session head with an Ed25519 key (AGEF v0.1.2).\n\n\
Reads the bundle, signs the canonical AGEF-SIG-v1 statement over its session head with the given \
PKCS#8 Ed25519 private key, appends the detached signature to manifest.signatures[], and writes \
the signed bundle (in place, or to --output). Prints the signer's public key as hex for \
distribution to verifiers (`akmon bundle verify --verify-key` / `agef-verify --verify-key`).\n\n\
Generate a key with: openssl genpkey -algorithm ed25519 -outform DER -out signer.pk8\n\n\
Examples:\n\
  akmon bundle sign audit.akmon --key signer.pk8\n\
  akmon bundle sign audit.akmon --key signer.pk8 --output signed.akmon\n\n\
Exit codes:\n\
  0 — bundle signed successfully\n\
  2 — usage error (unreadable or invalid Ed25519 key)\n\
  3 — I/O or environment error (bundle not found, malformed archive, write error)"
    )]
    Sign(BundleSignArgs),
    /// Emit artifacts to verify a bundle's signature with stock `openssl` alone (AGEF v0.1.2).
    #[command(
        long_about = "Emit the artifacts a third party needs to verify a bundle's Ed25519 \
signature with stock OpenSSL 3.x — no Akmon binary, no cloud (metric F.1).\n\n\
Reads the signed bundle (read-only), reconstructs the canonical AGEF-SIG-v1 statement over its \
session head, extracts the matching detached signature from manifest.signatures[], and writes \
three files into --out-dir: statement.bin (the exact signed message), signature.bin (the 64-byte \
raw signature), and pubkey.pem (the supplied public key in SPKI PEM form). It then prints the \
exact openssl command to run. This signs nothing and never modifies the bundle.\n\n\
The signature is selected by matching the supplied --verify-key (the signer's public key as 64 \
hex chars, the same artifact `akmon bundle sign` prints) against manifest.signatures[].key_id.\n\n\
NOTE: stock LibreSSL (macOS /usr/bin/openssl) cannot verify Ed25519; the verifier needs \
OpenSSL 3.x.\n\n\
Examples:\n\
  akmon bundle prove-openssl audit.akmon --verify-key signer.pub.hex\n\
  akmon bundle prove-openssl audit.akmon --verify-key signer.pub.hex --out-dir ./proof\n\
  akmon bundle prove-openssl audit.akmon --verify-key signer.pub.hex --format json\n\n\
Exit codes:\n\
  0 — artifacts written; printed openssl command is ready to run\n\
  1 — no signature matches the supplied key, or the signature is unsupported/malformed\n\
  3 — I/O or environment error (bundle/--verify-key unreadable, malformed archive, out-dir not writable)"
    )]
    ProveOpenssl(crate::bundle_prove::BundleProveArgs),
}

/// Arguments for `akmon bundle export`.
#[derive(Args, Debug, Clone)]
pub struct BundleExportArgs {
    /// Session UUID assigned at AgentSession construction.
    pub(crate) session_id: uuid::Uuid,
    /// Path where the bundle file will be written.
    ///
    /// If omitted, defaults to `<session-id>.akmon` in the current directory.
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,
    /// Path to the journal directory.
    ///
    /// Defaults to per-user journal location (`$XDG_STATE_HOME/akmon/journal`).
    #[arg(long)]
    pub(crate) journal: Option<PathBuf>,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleExportFormat,
}

/// Arguments for `akmon bundle import`.
#[derive(Args, Debug, Clone)]
pub struct BundleImportArgs {
    /// Path to the `.akmon` bundle file to import.
    pub(crate) bundle: PathBuf,
    /// Path to the journal directory.
    ///
    /// Defaults to per-user journal location (`$XDG_STATE_HOME/akmon/journal`).
    #[arg(long)]
    pub(crate) journal: Option<PathBuf>,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleImportFormat,
    /// Verify the bundle without modifying the local journal.
    ///
    /// When set, the bundle is fully validated per AGEF Sections 13 and 14 but no objects or
    /// events are written.
    #[arg(long)]
    pub(crate) verify_only: bool,
    /// Allow the import to succeed when the tar archive contains files outside the AGEF normative
    /// set (`manifest.json`, `events.bin`, `objects/<hex>`).
    ///
    /// Default is strict: unknown files cause hard reject.
    #[arg(long)]
    pub(crate) allow_extra_files: bool,
    /// Re-map the bundle's `session_id` to a different UUID during import.
    ///
    /// Useful when importing a bundle whose `session_id` already exists in the local journal.
    /// Required when the local journal already contains the bundle's `session_id`.
    #[arg(long, value_name = "NEW_UUID")]
    pub(crate) rename_to: Option<uuid::Uuid>,
}

/// Arguments for `akmon bundle verify`.
#[derive(Args, Debug, Clone)]
pub struct BundleVerifyArgs {
    /// Path to the `.akmon` bundle file to verify.
    bundle: PathBuf,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    format: BundleImportFormat,
    /// Allow the verify to succeed when the tar archive contains files outside the AGEF normative
    /// set (`manifest.json`, `events.bin`, `objects/<hex>`).
    ///
    /// Default is strict: unknown files cause hard reject.
    #[arg(long)]
    allow_extra_files: bool,
    /// Trusted Ed25519 public key as hex (64 chars), read from a file. Repeatable. When provided,
    /// each `manifest.signatures[]` entry is verified against these keys (AGEF v0.1.2).
    #[arg(long = "verify-key", value_name = "HEX_FILE")]
    verify_keys: Vec<PathBuf>,
    /// Fail (exit 1) unless at least one signature verifies against a `--verify-key`.
    #[arg(long)]
    require_signature: bool,
    /// Fail unless the session captured full message content (rejects metadata-only OTEL imports).
    #[arg(long, value_enum, value_name = "LEVEL")]
    require_capture: Option<RequireCapture>,
}

/// Arguments for `akmon bundle sign`.
#[derive(Args, Debug, Clone)]
pub struct BundleSignArgs {
    /// Path to the `.akmon` bundle file to sign.
    bundle: PathBuf,
    /// Path to a PKCS#8 Ed25519 private key (DER), e.g. from
    /// `openssl genpkey -algorithm ed25519 -outform DER`.
    #[arg(long)]
    key: PathBuf,
    /// Destination for the signed bundle. Defaults to signing the input bundle in place.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    format: BundleImportFormat,
}

pub(crate) fn format_bundle_byte_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub(crate) fn bundle_export_output_display(path: &Path) -> String {
    dunce::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn print_bundle_export_json_report(report: &BundleExportReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_bundle_export_json_error(category: &'static str, error: String) -> std::io::Result<()> {
    let body = BundleExportError {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        category: category.to_owned(),
        error,
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn bundle_export_error_category(msg: &str) -> &'static str {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("output path already exists") {
        "output_exists"
    } else if lower.contains("referenced object") && lower.contains("missing") {
        "missing_object"
    } else if lower.contains("malformed session") {
        "malformed_journal"
    } else if lower.contains("session not found") {
        "session_not_found"
    } else if lower.contains("redb open failed") || lower.contains("no such file or directory") {
        "journal_not_found"
    } else if lower.contains("bundle") || lower.contains("invalid manifest") {
        "bundle_error"
    } else {
        "io_error"
    }
}

fn run_bundle_export(
    session_id: uuid::Uuid,
    output: Option<PathBuf>,
    journal: Option<PathBuf>,
    format: BundleExportFormat,
) -> ExitCode {
    let journal_dir = match journal {
        Some(path) => path,
        None => match default_journal_dir() {
            Ok(path) => path,
            Err(err) => {
                let msg = format!("cannot resolve default journal directory: {err}");
                if matches!(format, BundleExportFormat::Json) {
                    let _ = print_bundle_export_json_error("journal_not_found", msg);
                } else {
                    eprintln!("akmon: bundle export: {msg}");
                }
                return ExitCode::from(3);
            }
        },
    };

    let output_path =
        output.unwrap_or_else(|| PathBuf::from(format!("{}.akmon", session_id.as_hyphenated())));

    if output_path.exists() {
        let msg = format!(
            "error: output path already exists: {}\nuse a different --output path or remove the existing file",
            output_path.display()
        );
        if matches!(format, BundleExportFormat::Json) {
            let _ = print_bundle_export_json_error(
                "output_exists",
                format!("output path already exists: {}", output_path.display()),
            );
        } else {
            eprintln!("{msg}");
        }
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
            if matches!(format, BundleExportFormat::Json) {
                let _ = print_bundle_export_json_error(bundle_export_error_category(&msg), msg);
            } else {
                eprintln!("akmon: bundle export: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    let (history, head_hash) = {
        let graph = handle
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let history = match graph.history() {
            Ok(h) => h,
            Err(err) => {
                let msg = format!("cannot read session history: {err}");
                if matches!(format, BundleExportFormat::Json) {
                    let _ = print_bundle_export_json_error("io_error", msg);
                } else {
                    eprintln!("akmon: bundle export: {msg}");
                }
                return ExitCode::from(3);
            }
        };
        let head = match graph.head() {
            Ok(h) => h,
            Err(err) => {
                let msg = format!("cannot read session head: {err}");
                if matches!(format, BundleExportFormat::Json) {
                    let _ = print_bundle_export_json_error("io_error", msg);
                } else {
                    eprintln!("akmon: bundle export: {msg}");
                }
                return ExitCode::from(3);
            }
        };
        let Some(head_hash) = head else {
            let msg = "malformed session: empty event graph (no head)".to_owned();
            if matches!(format, BundleExportFormat::Json) {
                let _ = print_bundle_export_json_error("malformed_journal", msg.clone());
            } else {
                eprintln!("akmon: bundle export: {msg}");
            }
            return ExitCode::from(3);
        };
        (history, head_hash)
    };

    let Some((_, start_event)) = history
        .iter()
        .find(|(_, e)| matches!(e.kind, EventKind::SessionStart { .. }))
    else {
        let msg =
            "malformed session: journal has no SessionStart event (cannot build bundle)".to_owned();
        if matches!(format, BundleExportFormat::Json) {
            let _ = print_bundle_export_json_error("malformed_journal", msg.clone());
        } else {
            eprintln!("akmon: bundle export: {msg}");
        }
        return ExitCode::from(3);
    };

    let Some((_, end_event)) = history
        .iter()
        .rev()
        .find(|(_, e)| matches!(e.kind, EventKind::SessionEnd { .. }))
    else {
        let msg =
            "malformed session: journal has no SessionEnd event (cannot build bundle)".to_owned();
        if matches!(format, BundleExportFormat::Json) {
            let _ = print_bundle_export_json_error("malformed_journal", msg.clone());
        } else {
            eprintln!("akmon: bundle export: {msg}");
        }
        return ExitCode::from(3);
    };

    let events: Vec<akmon_journal::Event> = history.iter().map(|(_, e)| e.clone()).collect();

    let mut objects: HashMap<akmon_journal::Hash, Vec<u8>> = HashMap::new();
    for (_, ev) in &history {
        for h in referenced_object_hashes_for_kind(&ev.kind) {
            if objects.contains_key(&h) {
                continue;
            }
            match handle.store.get(&h) {
                Ok(Some(bytes)) => {
                    objects.insert(h, bytes.to_vec());
                }
                Ok(None) => {
                    let msg = format!(
                        "referenced object {} is missing from the object store; journal is incomplete",
                        h.to_hex()
                    );
                    if matches!(format, BundleExportFormat::Json) {
                        let _ = print_bundle_export_json_error("missing_object", msg.clone());
                    } else {
                        eprintln!("akmon: bundle export: {msg}");
                    }
                    return ExitCode::from(3);
                }
                Err(err) => {
                    let msg = format!("object store read failed for {}: {err}", h.to_hex());
                    if matches!(format, BundleExportFormat::Json) {
                        let _ = print_bundle_export_json_error("io_error", msg.clone());
                    } else {
                        eprintln!("akmon: bundle export: {msg}");
                    }
                    return ExitCode::from(3);
                }
            }
        }
    }

    let created_at = crate::format_iso_utc(
        start_event.emitted_at.unix_timestamp(),
        start_event.emitted_at.nanosecond(),
    );
    let ended_at = crate::format_iso_utc(
        end_event.emitted_at.unix_timestamp(),
        end_event.emitted_at.nanosecond(),
    );

    let manifest = Manifest {
        agef_version: AGEF_SPEC_VERSION.to_owned(),
        producer: Producer {
            name: "akmon".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        session: SessionMetadata {
            id: session_id.as_hyphenated().to_string(),
            head: head_hash.to_hex(),
            created_at,
            ended_at,
        },
        hash_algorithm: handle.store.algorithm().to_string(),
        object_count: objects.len() as u64,
        event_count: events.len() as u64,
        signatures: None,
        extra: BTreeMap::new(),
    };

    let mut file = match std::fs::File::create(&output_path) {
        Ok(f) => f,
        Err(err) => {
            let msg = format!("cannot create bundle file {}: {err}", output_path.display());
            if matches!(format, BundleExportFormat::Json) {
                let _ = print_bundle_export_json_error("io_error", msg.clone());
            } else {
                eprintln!("akmon: bundle export: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    if let Err(err) = write_bundle(
        &mut file,
        &manifest,
        &events,
        &objects,
        &WriteBundleOptions::default(),
    ) {
        let msg = match err {
            BundleError::Io(ref e) => format!("bundle write I/O error: {e}"),
            other => format!("bundle write failed: {other}"),
        };
        let _ = std::fs::remove_file(&output_path);
        if matches!(format, BundleExportFormat::Json) {
            let _ = print_bundle_export_json_error("bundle_error", msg.clone());
        } else {
            eprintln!("akmon: bundle export: {msg}");
        }
        return ExitCode::from(3);
    }

    let size_bytes = match std::fs::metadata(&output_path) {
        Ok(m) => m.len(),
        Err(err) => {
            let msg = format!("cannot stat bundle file {}: {err}", output_path.display());
            if matches!(format, BundleExportFormat::Json) {
                let _ = print_bundle_export_json_error("io_error", msg.clone());
            } else {
                eprintln!("akmon: bundle export: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    match format {
        BundleExportFormat::Json => {
            let report = BundleExportReportV1 {
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                agef_version: AGEF_SPEC_VERSION.to_owned(),
                session_id: session_id.as_hyphenated().to_string(),
                output_path: bundle_export_output_display(output_path.as_path()),
                events_exported: events.len() as u64,
                objects_exported: objects.len() as u64,
                bundle_size_bytes: size_bytes,
            };
            if let Err(err) = print_bundle_export_json_report(&report) {
                eprintln!("akmon: bundle export: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        }
        BundleExportFormat::Human => {
            eprintln!("exported: session {session_id}");
            eprintln!("  events: {}", events.len());
            eprintln!("  objects: {}", objects.len());
            eprintln!(
                "  bundle: {}",
                bundle_export_output_display(output_path.as_path())
            );
            eprintln!("  size: {}", format_bundle_byte_size(size_bytes));
        }
    }

    ExitCode::SUCCESS
}
pub(crate) fn bundle_read_bundle_error_category(err: &BundleError) -> &'static str {
    match err {
        BundleError::Io(_) => "io_error",
        BundleError::InvalidArchive(_) => "invalid_archive",
        BundleError::InvalidCompression(_) => "invalid_compression",
        BundleError::InvalidManifest(_) => "invalid_manifest",
        BundleError::UnsupportedAgefVersion(_) => "unsupported_agef_version",
        BundleError::UnsupportedHashAlgorithm(_) => "unsupported_hash_algorithm",
        BundleError::MalformedFraming(_) => "malformed_framing",
        BundleError::FrameTooLarge(_) => "frame_too_large",
        BundleError::NonCanonicalCbor => "non_canonical_cbor",
        BundleError::UnknownEventKind(_) => "unknown_event_kind",
        BundleError::UnknownAttemptStatus(_) => "unknown_attempt_status",
        BundleError::MissingObject(_) => "missing_object",
        BundleError::ObjectHashMismatch(_) => "object_hash_mismatch",
        BundleError::HeadMismatch { .. } => "head_mismatch",
        BundleError::UnknownBundleFile(_) => "unknown_bundle_file",
    }
}

pub(crate) fn bundle_read_bundle_exit_code(err: &BundleError) -> u8 {
    match err {
        BundleError::Io(_) => 3,
        _ => 1,
    }
}

fn print_bundle_import_infra_json_error(
    category: &str,
    error: String,
    colliding_session_id: Option<uuid::Uuid>,
) -> std::io::Result<()> {
    let body = BundleImportInfraError {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        error,
        category: category.to_owned(),
        colliding_session_id: colliding_session_id.map(|u| u.as_hyphenated().to_string()),
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_bundle_verify_json_report(report: &BundleVerifyReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn print_bundle_import_json_report(report: &BundleImportReportV1) -> std::io::Result<()> {
    let json =
        serde_json::to_string_pretty(report).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}

fn cli_violations_from_bundle_verify(
    report: &akmon_bundle::BundleVerificationReport,
) -> Vec<BundleViolation> {
    report
        .violations
        .iter()
        .map(|v| BundleViolation {
            category: v.category().to_owned(),
            event_hash: v.event_hash_hex(),
            object_hash: v.object_hash_hex(),
            message: v.message(),
        })
        .collect()
}

fn manifest_hash_algorithm(manifest: &Manifest) -> Option<HashAlgorithm> {
    match manifest.hash_algorithm.as_str() {
        "sha256" => Some(HashAlgorithm::Sha256),
        "blake3" => Some(HashAlgorithm::Blake3),
        _ => None,
    }
}

fn bundle_import_history(
    contents: &BundleContents,
    algorithm: HashAlgorithm,
) -> Result<Vec<(Hash, Event)>, String> {
    let mut history = Vec::with_capacity(contents.events.len());
    for event in &contents.events {
        let hash = event
            .content_hash(algorithm)
            .map_err(|e| format!("event content hash failed: {e}"))?;
        history.push((hash, event.clone()));
    }
    Ok(history)
}

fn exit_bundle_verify_failed(
    bundle: &Path,
    contents: &BundleContents,
    format: BundleImportFormat,
    violations: Vec<BundleViolation>,
) -> ExitCode {
    let report =
        build_bundle_verify_report_v1(bundle, contents, false, violations, Vec::new(), None);
    match format {
        BundleImportFormat::Json => {
            if let Err(err) = print_bundle_verify_json_report(&report) {
                eprintln!("akmon: bundle import: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        }
        BundleImportFormat::Human => {
            let bundle_disp = bundle_export_output_display(bundle);
            eprintln!("bundle verification failed: {bundle_disp}");
            eprintln!("  session_id: {}", contents.manifest.session.id);
            eprintln!("  events in bundle: {}", contents.events.len());
            eprintln!("  objects in bundle: {}", contents.objects.len());
            eprintln!();
            eprintln!("  violations:");
            for v in &report.violations {
                eprintln!("    - [{}] {}", v.category, v.message);
            }
        }
    }
    ExitCode::from(1)
}

/// Category string used for the synthetic violation emitted when `--require-capture full` fails.
const CAPTURE_REQUIREMENT_CATEGORY: &str = "capture_requirement_unmet";

/// Whether an OTEL import satisfies `--require-capture full`.
///
/// Native bundles (`capture == None`) are full-fidelity and always pass. An OTEL import passes only
/// when its `capture_level` is exactly `"full"`. Returns `None` (no requirement) when
/// `require_capture` is unset.
fn capture_requirement_unmet(
    require_capture: Option<RequireCapture>,
    capture: Option<&OtelCaptureInfo>,
) -> Option<String> {
    match require_capture {
        None => None,
        Some(RequireCapture::Full) => match capture {
            None => None,
            Some(info) if info.capture_level == "full" => None,
            Some(info) => Some(format!(
                "--require-capture full: session capture level is '{}', not 'full' (metadata-only OTEL import; source telemetry did not capture message content)",
                info.capture_level
            )),
        },
    }
}

fn build_bundle_verify_report_v1(
    bundle_path: &Path,
    contents: &BundleContents,
    passed: bool,
    violations: Vec<BundleViolation>,
    signatures: Vec<CliSignatureReport>,
    capture: Option<&OtelCaptureInfo>,
) -> BundleVerifyReportV1 {
    BundleVerifyReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: contents.manifest.agef_version.clone(),
        bundle_path: bundle_export_output_display(bundle_path),
        session_id: contents.manifest.session.id.clone(),
        events_in_bundle: contents.events.len() as u64,
        objects_in_bundle: contents.objects.len() as u64,
        passed,
        violations,
        signatures,
        capture: capture.map(|info| CliCaptureField {
            level: info.capture_level.clone(),
            source_semconv: info.source_semconv.clone(),
        }),
    }
}

/// Short suffix appended to the success headline so a structural import never reads as a bare
/// "verified bundle". Native bundles (`None`) and full OTEL imports add nothing.
fn capture_human_suffix(capture: Option<&CliCaptureField>) -> String {
    match capture {
        Some(c) if c.level == "structural" => " — capture: STRUCTURAL (metadata only)".to_owned(),
        _ => String::new(),
    }
}

/// Prominent capture line for the human report body, or `None` for native (non-OTEL) bundles.
fn capture_human_line(capture: Option<&CliCaptureField>) -> Option<String> {
    let c = capture?;
    match c.level.as_str() {
        "structural" => Some(format!(
            "capture: STRUCTURAL (metadata only — message content was NOT captured by the source telemetry; source semconv {})",
            c.source_semconv
        )),
        "full" => Some("capture: FULL (OTEL import; message content captured)".to_owned()),
        other => Some(format!(
            "capture: {} (OTEL import; source semconv {})",
            other.to_uppercase(),
            c.source_semconv
        )),
    }
}

/// Verifies a bundle file without journal access (Item 4.3 / F4).
#[allow(clippy::too_many_arguments)]
fn run_bundle_verify(
    bundle: PathBuf,
    format: BundleImportFormat,
    allow_extra_files: bool,
    verify_keys: Vec<PathBuf>,
    require_signature: bool,
    require_capture: Option<RequireCapture>,
    label: &'static str,
) -> ExitCode {
    let json = matches!(format, BundleImportFormat::Json);
    let mut file = match std::fs::File::open(&bundle) {
        Ok(f) => f,
        Err(err) => {
            let msg = format!("cannot open bundle {}: {err}", bundle.display());
            if json {
                let _ = print_bundle_import_infra_json_error("io_error", msg.clone(), None);
            } else {
                eprintln!("akmon: {label}: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    let options = ReadBundleOptions {
        allow_extra_files,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };

    let contents = match read_bundle(&mut file, &options) {
        Ok(c) => c,
        Err(err) => {
            let msg = err.to_string();
            let category = bundle_read_bundle_error_category(&err);
            let code = bundle_read_bundle_exit_code(&err);
            if json {
                let _ = print_bundle_import_infra_json_error(category, msg.clone(), None);
            } else {
                eprintln!("akmon: {label}: {msg}");
            }
            return ExitCode::from(code);
        }
    };

    // Load any trusted public keys (hex -> raw 32 bytes) before signature verification.
    let mut trusted_keys = Vec::with_capacity(verify_keys.len());
    for key_path in &verify_keys {
        let parsed = std::fs::read_to_string(key_path)
            .map_err(|err| format!("cannot read --verify-key {}: {err}", key_path.display()))
            .and_then(|hex_str| {
                parse_public_key_hex(&hex_str)
                    .map_err(|err| format!("--verify-key {}: {err}", key_path.display()))
            });
        match parsed {
            Ok(key) => trusted_keys.push(key),
            Err(msg) => {
                if json {
                    let _ = print_bundle_import_infra_json_error("verify_key_error", msg, None);
                } else {
                    eprintln!("akmon: {label}: {msg}");
                }
                return ExitCode::from(3);
            }
        }
    }
    let sig_report = verify_manifest_signatures(&contents.manifest, &trusted_keys);
    let verification = verify_bundle(&contents);
    let capture = otel_capture_info(&contents);
    let signatures_ok =
        !sig_report.any_invalid() && (!require_signature || sig_report.any_verified());
    let capture_unmet = capture_requirement_unmet(require_capture, capture.as_ref());
    let passed = verification.is_clean() && signatures_ok && capture_unmet.is_none();
    let mut violations = cli_violations_from_bundle_verify(&verification);
    if let Some(reason) = &capture_unmet {
        violations.push(BundleViolation {
            category: CAPTURE_REQUIREMENT_CATEGORY.to_owned(),
            event_hash: None,
            object_hash: None,
            message: reason.clone(),
        });
    }
    let report = build_bundle_verify_report_v1(
        &bundle,
        &contents,
        passed,
        violations,
        cli_signature_reports(&sig_report),
        capture.as_ref(),
    );

    match format {
        BundleImportFormat::Json => {
            if let Err(err) = print_bundle_verify_json_report(&report) {
                eprintln!("akmon: {label}: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        }
        BundleImportFormat::Human => {
            let bundle_disp = bundle_export_output_display(bundle.as_path());
            let capture_suffix = capture_human_suffix(report.capture.as_ref());
            if passed {
                eprintln!("verified bundle (integrity + signature){capture_suffix}: {bundle_disp}");
            } else {
                eprintln!("bundle verification FAILED: {bundle_disp}");
            }
            eprintln!("  session_id: {}", contents.manifest.session.id);
            eprintln!("  events: {}", contents.events.len());
            eprintln!("  objects: {}", contents.objects.len());
            if let Some(line) = capture_human_line(report.capture.as_ref()) {
                eprintln!("  {line}");
            }
            if !report.violations.is_empty() {
                eprintln!("  integrity violations:");
                for v in &report.violations {
                    eprintln!("    - [{}] {}", v.category, v.message);
                }
            }
            if report.signatures.is_empty() {
                eprintln!("  signatures: none");
            } else {
                eprintln!("  signatures:");
                for s in &report.signatures {
                    eprintln!("    - {} [{}] key_id={}", s.outcome, s.scheme, s.key_id);
                }
            }
        }
    }

    if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn run_bundle_import(
    bundle: PathBuf,
    journal: Option<PathBuf>,
    format: BundleImportFormat,
    verify_only: bool,
    allow_extra_files: bool,
    rename_to: Option<uuid::Uuid>,
) -> ExitCode {
    if verify_only {
        return run_bundle_verify(
            bundle,
            format,
            allow_extra_files,
            Vec::new(),
            false,
            None,
            "bundle import",
        );
    }

    let journal_dir = match journal {
        Some(path) => path,
        None => match default_journal_dir() {
            Ok(path) => path,
            Err(err) => {
                let msg = format!("cannot resolve default journal directory: {err}");
                if matches!(format, BundleImportFormat::Json) {
                    let _ = print_bundle_import_infra_json_error(
                        "journal_not_found",
                        msg.clone(),
                        None,
                    );
                } else {
                    eprintln!("akmon: bundle import: {msg}");
                }
                return ExitCode::from(3);
            }
        },
    };

    let mut file = match std::fs::File::open(&bundle) {
        Ok(f) => f,
        Err(err) => {
            let msg = format!("cannot open bundle {}: {err}", bundle.display());
            if matches!(format, BundleImportFormat::Json) {
                let _ = print_bundle_import_infra_json_error("io_error", msg.clone(), None);
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(3);
        }
    };

    let options = ReadBundleOptions {
        allow_extra_files,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };

    let contents = match read_bundle(&mut file, &options) {
        Ok(c) => c,
        Err(err) => {
            let msg = err.to_string();
            let category = bundle_read_bundle_error_category(&err);
            let code = bundle_read_bundle_exit_code(&err);
            if matches!(format, BundleImportFormat::Json) {
                let _ = print_bundle_import_infra_json_error(category, msg.clone(), None);
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(code);
        }
    };

    let bundle_sid = match uuid::Uuid::parse_str(contents.manifest.session.id.trim()) {
        Ok(u) => u,
        Err(err) => {
            let msg = format!("bundle manifest session id is not a valid UUID: {err}");
            if matches!(format, BundleImportFormat::Json) {
                let _ = print_bundle_import_infra_json_error(
                    "invalid_manifest_session_id",
                    msg.clone(),
                    None,
                );
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(1);
        }
    };

    let target_session_id = rename_to.unwrap_or(bundle_sid);

    match journal_contains_session(&journal_dir, target_session_id) {
        Ok(true) => {
            let msg = format!("target journal already contains session {target_session_id}");
            if matches!(format, BundleImportFormat::Json) {
                let _ = print_bundle_import_infra_json_error(
                    "session_id_collision",
                    msg,
                    Some(target_session_id),
                );
            } else {
                eprintln!("akmon: bundle import: error: {msg}");
                if rename_to.is_none() {
                    eprintln!(
                        "akmon: bundle import: hint: use --rename-to <NEW_UUID> to import as a different session"
                    );
                } else {
                    eprintln!(
                        "akmon: bundle import: hint: --rename-to target also collides; choose a different UUID"
                    );
                }
            }
            return ExitCode::from(2);
        }
        Ok(false) => {}
        Err(err) => {
            let msg = err.to_string();
            if matches!(format, BundleImportFormat::Json) {
                let _ = print_bundle_import_infra_json_error("journal_access", msg.clone(), None);
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(3);
        }
    }

    let verification = verify_bundle(&contents);
    if !verification.is_clean() {
        return exit_bundle_verify_failed(
            bundle.as_path(),
            &contents,
            format,
            cli_violations_from_bundle_verify(&verification),
        );
    }

    let Some(algorithm) = manifest_hash_algorithm(&contents.manifest) else {
        let msg = format!(
            "unsupported hash algorithm in manifest: {}",
            contents.manifest.hash_algorithm
        );
        if matches!(format, BundleImportFormat::Json) {
            let _ = print_bundle_import_infra_json_error(
                "unsupported_hash_algorithm",
                msg.clone(),
                None,
            );
        } else {
            eprintln!("akmon: bundle import: {msg}");
        }
        return ExitCode::from(1);
    };

    let history = match bundle_import_history(&contents, algorithm) {
        Ok(h) => h,
        Err(msg) => {
            if matches!(format, BundleImportFormat::Json) {
                let _ =
                    print_bundle_import_infra_json_error("verification_error", msg.clone(), None);
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(1);
        }
    };

    if let Err(err) = std::fs::create_dir_all(journal_dir.as_path()) {
        let msg = format!(
            "cannot create journal directory {}: {err}",
            journal_dir.display()
        );
        if matches!(format, BundleImportFormat::Json) {
            let _ = print_bundle_import_infra_json_error("journal_access", msg.clone(), None);
        } else {
            eprintln!("akmon: bundle import: {msg}");
        }
        return ExitCode::from(3);
    }
    let journal_db = journal_dir.join("journal.redb");
    let store = if journal_db.is_file() {
        match RedbObjectStore::open(journal_db.as_path()) {
            Ok(s) => Arc::new(s),
            Err(err) => {
                let msg = format!("cannot open journal store {}: {err}", journal_db.display());
                if matches!(format, BundleImportFormat::Json) {
                    let _ =
                        print_bundle_import_infra_json_error("journal_access", msg.clone(), None);
                } else {
                    eprintln!("akmon: bundle import: {msg}");
                }
                return ExitCode::from(3);
            }
        }
    } else {
        match RedbObjectStore::create(journal_db.as_path(), algorithm) {
            Ok(s) => Arc::new(s),
            Err(err) => {
                let msg = format!(
                    "cannot create journal store {}: {err}",
                    journal_db.display()
                );
                if matches!(format, BundleImportFormat::Json) {
                    let _ =
                        print_bundle_import_infra_json_error("journal_access", msg.clone(), None);
                } else {
                    eprintln!("akmon: bundle import: {msg}");
                }
                return ExitCode::from(3);
            }
        }
    };

    let mut objects_new: u64 = 0;
    let mut objects_existing: u64 = 0;
    for (hash, bytes) in &contents.objects {
        match store.contains(hash) {
            Ok(true) => {
                let existing = match store.get(hash) {
                    Ok(Some(b)) => b,
                    Ok(None) => {
                        let msg = format!(
                            "local store returned contains=true but missing object {}",
                            hash.to_hex()
                        );
                        if matches!(format, BundleImportFormat::Json) {
                            let _ = print_bundle_import_infra_json_error(
                                "local_store_corrupt",
                                msg.clone(),
                                None,
                            );
                        } else {
                            eprintln!("akmon: bundle import: {msg}");
                        }
                        return ExitCode::from(3);
                    }
                    Err(err) => {
                        let msg = format!("cannot read existing object {}: {err}", hash.to_hex());
                        if matches!(format, BundleImportFormat::Json) {
                            let _ = print_bundle_import_infra_json_error(
                                "journal_access",
                                msg.clone(),
                                None,
                            );
                        } else {
                            eprintln!("akmon: bundle import: {msg}");
                        }
                        return ExitCode::from(3);
                    }
                };
                if existing.as_ref() != bytes.as_slice() {
                    let msg = format!(
                        "local object bytes mismatch for hash {}; refusing import (local_store_corrupt)",
                        hash.to_hex()
                    );
                    if matches!(format, BundleImportFormat::Json) {
                        let _ = print_bundle_import_infra_json_error(
                            "local_store_corrupt",
                            msg.clone(),
                            None,
                        );
                    } else {
                        eprintln!("akmon: bundle import: {msg}");
                    }
                    return ExitCode::from(3);
                }
                objects_existing += 1;
            }
            Ok(false) => {
                if let Err(err) = store.put(bytes.as_slice()) {
                    let msg = format!("cannot write object {}: {err}", hash.to_hex());
                    if matches!(format, BundleImportFormat::Json) {
                        let _ = print_bundle_import_infra_json_error(
                            "journal_access",
                            msg.clone(),
                            None,
                        );
                    } else {
                        eprintln!("akmon: bundle import: {msg}");
                    }
                    return ExitCode::from(3);
                }
                objects_new += 1;
            }
            Err(err) => {
                let msg = format!("cannot probe existing object {}: {err}", hash.to_hex());
                if matches!(format, BundleImportFormat::Json) {
                    let _ =
                        print_bundle_import_infra_json_error("journal_access", msg.clone(), None);
                } else {
                    eprintln!("akmon: bundle import: {msg}");
                }
                return ExitCode::from(3);
            }
        }
    }
    // NOTE(Item 4.3 layer 5b-3): object staging and graph import are intentionally not a
    // single transaction. Failures can leave orphan objects, which are content-addressed and
    // harmless; a future GC command may reclaim them.

    let mut graph = match RedbSessionGraph::open_new(Arc::clone(&store), target_session_id) {
        Ok(g) => g,
        Err(err) => {
            let msg = format!("cannot open target session {target_session_id}: {err}");
            if matches!(format, BundleImportFormat::Json) {
                let _ =
                    print_bundle_import_infra_json_error("session_open_failed", msg.clone(), None);
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(3);
        }
    };
    if let Err(err) = graph.import_verified_linear_history(&history) {
        let msg = format!("cannot import event history for session {target_session_id}: {err}");
        if matches!(format, BundleImportFormat::Json) {
            let _ = print_bundle_import_infra_json_error("import_failed", msg.clone(), None);
        } else {
            eprintln!("akmon: bundle import: {msg}");
        }
        return ExitCode::from(3);
    }

    let post = match graph.verify() {
        Ok(r) => r,
        Err(err) => {
            let msg = format!(
                "post-import verification errored for session {target_session_id}: {err}; manual cleanup may be required"
            );
            if matches!(format, BundleImportFormat::Json) {
                let _ = print_bundle_import_infra_json_error(
                    "post_import_verify_failed",
                    msg.clone(),
                    Some(target_session_id),
                );
            } else {
                eprintln!("akmon: bundle import: {msg}");
            }
            return ExitCode::from(3);
        }
    };
    if !post.is_clean() {
        let msg = format!(
            "post-import verification failed for session {target_session_id}; journal may contain a broken imported session and may require manual cleanup"
        );
        if matches!(format, BundleImportFormat::Json) {
            let _ = print_bundle_import_infra_json_error(
                "post_import_verify_failed",
                msg.clone(),
                Some(target_session_id),
            );
        } else {
            eprintln!("akmon: bundle import: {msg}");
        }
        return ExitCode::from(3);
    }

    let report = BundleImportReportV1 {
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        agef_version: contents.manifest.agef_version.clone(),
        bundle_path: bundle_export_output_display(bundle.as_path()),
        original_session_id: contents.manifest.session.id.clone(),
        imported_session_id: target_session_id.as_hyphenated().to_string(),
        events_imported: contents.events.len() as u64,
        objects_total: contents.objects.len() as u64,
        objects_new,
        objects_existing,
        journal_path: bundle_export_output_display(journal_dir.as_path()),
    };
    match format {
        BundleImportFormat::Json => {
            if let Err(err) = print_bundle_import_json_report(&report) {
                eprintln!("akmon: bundle import: failed to render JSON output: {err}");
                return ExitCode::from(3);
            }
        }
        BundleImportFormat::Human => {
            eprintln!("imported bundle: {}", report.bundle_path);
            eprintln!("  original session: {}", report.original_session_id);
            if report.original_session_id == report.imported_session_id {
                eprintln!(
                    "  imported as: {} (same as original)",
                    report.imported_session_id
                );
            } else {
                eprintln!("  imported as: {} (renamed)", report.imported_session_id);
            }
            eprintln!("  events: {}", report.events_imported);
            eprintln!(
                "  objects: {} ({} new, {} existing in store)",
                report.objects_total, report.objects_new, report.objects_existing
            );
            eprintln!("  target journal: {}", report.journal_path);
        }
    }
    ExitCode::SUCCESS
}

/// Runs one `akmon bundle` subcommand.
/// Stable JSON shape for `akmon bundle sign --format json`.
#[derive(Debug, Serialize, Deserialize)]
struct BundleSignReportV1 {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version.
    akmon_version: String,
    /// Path the signed bundle was written to.
    bundle_path: String,
    /// Session UUID from the bundle manifest.
    session_id: String,
    /// Signature scheme (`ed25519`).
    scheme: String,
    /// Signer key id (hex SHA-256 of the public key).
    key_id: String,
    /// Signer public key as lowercase hex (distribute this to verifiers).
    public_key_hex: String,
    /// Total signatures on the bundle after signing.
    signature_count: usize,
}

/// Reads a bundle, signs its session head with an Ed25519 PKCS#8 key, and writes the signed bundle.
fn run_bundle_sign(
    bundle: PathBuf,
    key_path: PathBuf,
    output: Option<PathBuf>,
    format: BundleImportFormat,
) -> ExitCode {
    let json = matches!(format, BundleImportFormat::Json);
    let fail = |category: &str, msg: String, code: u8| -> ExitCode {
        if json {
            let _ = print_bundle_import_infra_json_error(category, msg, None);
        } else {
            eprintln!("akmon: bundle sign: {msg}");
        }
        ExitCode::from(code)
    };

    let mut file = match std::fs::File::open(&bundle) {
        Ok(f) => f,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot open bundle {}: {err}", bundle.display()),
                3,
            );
        }
    };
    let options = ReadBundleOptions {
        allow_extra_files: false,
        max_event_frame_len: DEFAULT_MAX_EVENT_FRAME_LEN,
    };
    let mut contents = match read_bundle(&mut file, &options) {
        Ok(c) => c,
        Err(err) => {
            let category = bundle_read_bundle_error_category(&err);
            let code = bundle_read_bundle_exit_code(&err);
            return fail(category, err.to_string(), code);
        }
    };
    drop(file);

    let pkcs8 = match std::fs::read(&key_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot read --key {}: {err}", key_path.display()),
                3,
            );
        }
    };
    let public_key = match public_key_from_pkcs8(&pkcs8) {
        Ok(pk) => pk,
        Err(err) => {
            return fail(
                "invalid_key",
                format!("--key {}: {err}", key_path.display()),
                2,
            );
        }
    };

    // A signed bundle declares AGEF v0.1.2, the version that defines manifest.signatures[].
    contents.manifest.agef_version = AGEF_SPEC_VERSION.to_owned();
    let statement = {
        let m = &contents.manifest;
        signing_statement(
            &m.agef_version,
            &m.hash_algorithm,
            &m.session.id,
            &m.session.head,
        )
    };
    let signature = match sign_statement(statement.as_bytes(), &pkcs8) {
        Ok(sig) => sig,
        Err(err) => return fail("invalid_key", format!("signing failed: {err}"), 2),
    };
    let created_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let public_key_hex = hex::encode(&public_key);
    let kid = key_id(&public_key);
    contents
        .manifest
        .signatures
        .get_or_insert_with(Vec::new)
        .push(ManifestSignature {
            scheme: SCHEME_ED25519.to_owned(),
            key_id: kid.clone(),
            statement_version: SIG_STATEMENT_VERSION.to_owned(),
            signature: hex::encode(&signature),
            created_at,
        });

    // Write atomically (temp + rename) so a failed write never clobbers the input bundle.
    let out_path = output.unwrap_or_else(|| bundle.clone());
    let tmp_path = out_path.with_extension("akmon-signing-tmp");
    let mut out_file = match std::fs::File::create(&tmp_path) {
        Ok(f) => f,
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot create temp file {}: {err}", tmp_path.display()),
                3,
            );
        }
    };
    if let Err(err) = write_bundle(
        &mut out_file,
        &contents.manifest,
        &contents.events,
        &contents.objects,
        &WriteBundleOptions::default(),
    ) {
        drop(out_file);
        let _ = std::fs::remove_file(&tmp_path);
        return fail("bundle_error", format!("bundle write failed: {err}"), 3);
    }
    drop(out_file);
    if let Err(err) = std::fs::rename(&tmp_path, &out_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return fail(
            "io_error",
            format!(
                "cannot finalize signed bundle {}: {err}",
                out_path.display()
            ),
            3,
        );
    }

    let signature_count = contents.manifest.signatures.as_ref().map_or(0, Vec::len);
    match format {
        BundleImportFormat::Json => {
            let report = BundleSignReportV1 {
                tool: "akmon".to_owned(),
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                bundle_path: out_path.display().to_string(),
                session_id: contents.manifest.session.id.clone(),
                scheme: SCHEME_ED25519.to_owned(),
                key_id: kid,
                public_key_hex,
                signature_count,
            };
            match serde_json::to_string_pretty(&report) {
                Ok(s) => println!("{s}"),
                Err(err) => {
                    return fail(
                        "io_error",
                        format!("failed to render JSON output: {err}"),
                        3,
                    );
                }
            }
        }
        BundleImportFormat::Human => {
            let disp = bundle_export_output_display(out_path.as_path());
            eprintln!("signed bundle: {disp}");
            eprintln!("  session_id: {}", contents.manifest.session.id);
            eprintln!("  scheme: {SCHEME_ED25519}");
            eprintln!("  key_id: {kid}");
            eprintln!("  public key (hex): {public_key_hex}");
            eprintln!("  signatures: {signature_count}");
            eprintln!("  to verify, distribute the public key (hex above) and run:");
            eprintln!("    agef-verify {disp} --verify-key <file-containing-the-public-key-hex>");
        }
    }
    ExitCode::SUCCESS
}

pub fn run_bundle(args: &BundleArgs) -> ExitCode {
    match &args.command {
        BundleCommands::Export(export_args) => run_bundle_export(
            export_args.session_id,
            export_args.output.clone(),
            export_args.journal.clone(),
            export_args.format,
        ),
        BundleCommands::Import(import_args) => run_bundle_import(
            import_args.bundle.clone(),
            import_args.journal.clone(),
            import_args.format,
            import_args.verify_only,
            import_args.allow_extra_files,
            import_args.rename_to,
        ),
        BundleCommands::Verify(verify_args) => run_bundle_verify(
            verify_args.bundle.clone(),
            verify_args.format,
            verify_args.allow_extra_files,
            verify_args.verify_keys.clone(),
            verify_args.require_signature,
            verify_args.require_capture,
            "bundle verify",
        ),
        BundleCommands::Sign(sign_args) => run_bundle_sign(
            sign_args.bundle.clone(),
            sign_args.key.clone(),
            sign_args.output.clone(),
            sign_args.format,
        ),
        BundleCommands::ProveOpenssl(prove_args) => {
            crate::bundle_prove::run_bundle_prove_openssl(prove_args)
        }
    }
}
