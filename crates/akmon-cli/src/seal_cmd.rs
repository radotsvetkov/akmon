//! `akmon seal` — one command from an OTEL trace (or an existing journal session) to a signed,
//! self-verified AGEF bundle a stranger can verify offline.
//!
//! `seal` is a composition, not a new trust path. Every step reuses the code the individual
//! commands run:
//!
//! 1. import-or-select — [`crate::otel_cmd::import_trace_into_journal`] (capture level recorded
//!    honestly: structural / partial / full) or an existing session id;
//! 2. bundle export — [`crate::bundle_cmd::assemble_session_bundle`];
//! 3. integrity verify — [`crate::bundle_cmd::read_bundle_verified_for_signing`] (the gate
//!    `bundle sign` uses);
//! 4. Ed25519 head signature — [`crate::bundle_cmd::add_head_signature`];
//! 5. optional operator attestation — [`crate::bundle_attest::add_operator_attestation`];
//! 6. self-verify with the derived public key file(s) —
//!    [`crate::bundle_cmd::read_and_evaluate_bundle`] with the same [`VerifyPolicy`] that
//!    `bundle verify --verify-key <pub> --require-signature [--require-operator-key <op.pub>]` builds;
//! 7. optional `prove-openssl` artifacts — [`crate::bundle_prove::emit_openssl_artifacts`].
//!
//! Outputs are written atomically: everything lands in temp files in the output directory and is
//! renamed into place only after self-verification passed. On any failure no bundle, no `.pub`
//! file, and no proof artifact is left behind.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_bundle::report::{ToolErrorV1, VerifyPolicy};
use akmon_bundle::{
    DEFAULT_MAX_BUNDLE_DECODED_BYTES, OperatorIdentity, WriteBundleOptions, key_id,
    public_key_from_pkcs8, sanitize_display, validate_operator_field, write_bundle,
};
use akmon_otel::{CaptureLevel, DEFAULT_MAX_TRACE_BYTES, ImportOptions};
use akmon_query::{default_journal_dir, journal_contains_session};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::bundle_attest::add_operator_attestation;
use crate::bundle_cmd::{
    BundleImportFormat, SessionBundle, add_head_signature, assemble_session_bundle,
    bundle_export_output_display, read_and_evaluate_bundle, read_bundle_verified_for_signing,
};
use crate::bundle_keygen::write_public_key_file;
use crate::bundle_prove::{emit_openssl_artifacts, proof_file_names};
use crate::otel_cmd::{OtelImportFailure, import_trace_into_journal, shell_quote};

/// Tool name used in the JSON success object and in `ToolErrorV1`.
const TOOL: &str = "akmon seal";

/// Arguments for `akmon seal`.
#[derive(Args, Debug, Clone)]
pub struct SealArgs {
    /// OTLP/JSON OpenTelemetry GenAI trace file, or an existing journal session id (UUID).
    #[arg(value_name = "INPUT")]
    pub(crate) input: String,
    /// Signer private key: PKCS#8 v2 Ed25519 (raw DER) from `akmon bundle keygen --out`.
    #[arg(long, value_name = "SIGNER_PK8")]
    pub(crate) key: PathBuf,
    /// Destination bundle path. Defaults to `./<session-id>.akmon`. The signer public key is
    /// written next to it as `<OUTPUT>.pub` (and `<OUTPUT>.operator.pub` when attesting).
    #[arg(long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,
    /// Journal directory to import into / read the session from.
    ///
    /// Defaults to the per-user journal location (`$XDG_STATE_HOME/akmon/journal`), the same one
    /// `akmon otel import` uses.
    #[arg(long, value_name = "DIR")]
    pub(crate) journal: Option<PathBuf>,
    /// Import only the spans of this trace id (trace input only; required when the file holds
    /// spans from more than one trace).
    #[arg(long, value_name = "HEX")]
    pub(crate) trace_id: Option<String>,
    /// Operator private key (PKCS#8 v2 Ed25519) for a signed operator attestation (AGEF-OPERATOR-v1).
    #[arg(long = "operator-key", value_name = "OP_PK8", requires = "operator_id")]
    pub(crate) operator_key: Option<PathBuf>,
    /// Stable operator identifier (signed), e.g. an email or employee id. Requires --operator-key.
    #[arg(long = "operator-id", value_name = "ID", requires = "operator_key")]
    pub(crate) operator_id: Option<String>,
    /// Operator display name (signed). Requires --operator-key.
    #[arg(long = "display-name", value_name = "S", requires = "operator_key")]
    pub(crate) display_name: Option<String>,
    /// Role the operator acted in, e.g. `approver` (signed). Requires --operator-key.
    #[arg(long, value_name = "S", requires = "operator_key")]
    pub(crate) role: Option<String>,
    /// Operator organization (signed). Requires --operator-key.
    #[arg(long, value_name = "S", requires = "operator_key")]
    pub(crate) org: Option<String>,
    /// Also write the `prove-openssl` artifacts (statement.bin, signature.bin, pubkey.pem, plus the
    /// operator_* files when attesting) into this directory.
    #[arg(long = "proof-dir", value_name = "DIR")]
    pub(crate) proof_dir: Option<PathBuf>,
    /// Overwrite an existing output bundle and its `.pub` file(s).
    #[arg(long)]
    pub(crate) force: bool,
    /// Output format: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleImportFormat,
}

/// Stable JSON shape for `akmon seal --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct SealReportV1 {
    /// Producer tool name (`akmon seal`).
    tool: String,
    /// Akmon crate version that produced this report.
    akmon_version: String,
    /// Always `true` in a success report: the sealed bundle passed self-verification.
    passed: bool,
    /// Hyphenated session UUID of the sealed session.
    session_id: String,
    /// Path of the sealed bundle (canonicalized for display).
    bundle: String,
    /// Signer public key as 64 lowercase hex characters (also written to `<bundle>.pub`).
    signer_public_key: String,
    /// Signer key id: lowercase hex SHA-256 of the signer public key.
    signer_key_id: String,
    /// Operator public key as 64 lowercase hex characters, when an attestation was added.
    #[serde(skip_serializing_if = "Option::is_none")]
    operator_public_key: Option<String>,
    /// Capture level of the sealed session: `full`, `partial`, or `structural`.
    capture_level: String,
    /// Directory the `prove-openssl` artifacts were written to, when `--proof-dir` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    proof_dir: Option<String>,
    /// The exact `agef-verify` command an auditor runs on the handed-over files.
    verify_command: String,
}

/// A refusal: stable category, message, exit code (1 verify, 2 usage/input, 3 I/O/environment).
#[derive(Debug)]
struct SealError {
    category: String,
    message: String,
    code: u8,
}

fn err(category: &str, message: String, code: u8) -> SealError {
    SealError {
        category: category.to_owned(),
        message,
        code,
    }
}

impl From<(&'static str, String, u8)> for SealError {
    fn from((category, message, code): (&'static str, String, u8)) -> Self {
        err(category, message, code)
    }
}

impl From<(String, String, u8)> for SealError {
    fn from((category, message, code): (String, String, u8)) -> Self {
        Self {
            category,
            message,
            code,
        }
    }
}

impl From<OtelImportFailure> for SealError {
    fn from(failure: OtelImportFailure) -> Self {
        err(failure.category, failure.message, failure.exit_code)
    }
}

/// What INPUT names.
enum SealInput {
    /// An OTLP/JSON trace file to import.
    Trace(PathBuf),
    /// An existing journal session.
    Session(uuid::Uuid),
}

/// A loaded Ed25519 key pair half: the private PKCS#8 bytes and the derived public key.
struct LoadedKey {
    pkcs8: Vec<u8>,
    public_key: Vec<u8>,
}

/// Everything a successful seal produced, for rendering.
struct SealOutcome {
    session_id: String,
    output: PathBuf,
    signer: Vec<u8>,
    operator: Option<Vec<u8>>,
    capture_level: String,
    proof_dir: Option<PathBuf>,
    openssl_commands: Vec<String>,
    imported: bool,
}

/// Removes every registered path on drop unless disarmed: the "no partial output" guarantee.
#[derive(Default)]
struct Cleanup {
    paths: Vec<PathBuf>,
    disarmed: bool,
}

impl Cleanup {
    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `<path><suffix>` (suffix appended to the full file name, e.g. `audit.akmon` -> `audit.akmon.pub`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut raw: OsString = path.as_os_str().to_owned();
    raw.push(suffix);
    PathBuf::from(raw)
}

/// A unique hidden temp path next to `final_path` (same directory, so `rename` is atomic).
fn temp_path_for(final_path: &Path, tag: &str) -> PathBuf {
    let dir = final_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bundle".to_owned());
    let nanos = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    dir.join(format!(
        ".{name}.{tag}.{}-{nanos}.seal-tmp",
        std::process::id()
    ))
}

/// Classifies INPUT: an existing file is a trace; otherwise it must parse as a session UUID.
fn classify_input(raw: &str) -> Result<SealInput, SealError> {
    let path = PathBuf::from(raw);
    if path.is_file() {
        return Ok(SealInput::Trace(path));
    }
    if let Ok(sid) = uuid::Uuid::parse_str(raw.trim()) {
        return Ok(SealInput::Session(sid));
    }
    if path.exists() {
        return Err(err(
            "invalid_input",
            format!(
                "input {raw} is not a regular file (expected an OTLP/JSON trace file or a session id)"
            ),
            2,
        ));
    }
    Err(err(
        "input_not_found",
        format!("input {raw} is neither an existing trace file nor a session id (UUID)"),
        2,
    ))
}

/// Reads a PKCS#8 v2 Ed25519 private key (I/O failure: exit 3; not a valid key: exit 2).
fn load_private_key(path: &Path, flag: &str) -> Result<LoadedKey, SealError> {
    let pkcs8 = std::fs::read(path).map_err(|e| {
        err(
            "io_error",
            format!("cannot read {flag} {}: {e}", path.display()),
            3,
        )
    })?;
    let public_key = public_key_from_pkcs8(&pkcs8)
        .map_err(|e| err("invalid_key", format!("{flag} {}: {e}", path.display()), 2))?;
    Ok(LoadedKey { pkcs8, public_key })
}

/// The operator identity, validated before any work is done (same rules as `bundle attest`).
fn operator_identity(args: &SealArgs) -> Result<Option<OperatorIdentity>, SealError> {
    let Some(operator_id) = args.operator_id.clone() else {
        return Ok(None);
    };
    let identity = OperatorIdentity {
        operator_id,
        display_name: args.display_name.clone().unwrap_or_default(),
        role: args.role.clone().unwrap_or_default(),
        org: args.org.clone().unwrap_or_default(),
    };
    let illegal = identity.operator_id.is_empty()
        || [
            &identity.operator_id,
            &identity.display_name,
            &identity.role,
            &identity.org,
        ]
        .into_iter()
        .any(|field| validate_operator_field(field).is_err());
    if illegal {
        return Err(err(
            "invalid_operator_field",
            "operator identity field is empty (operator-id) or contains a newline/carriage return; \
             such values are rejected to prevent statement injection"
                .to_owned(),
            2,
        ));
    }
    Ok(Some(identity))
}

/// Refuses (exit 2) when any final output already exists and `--force` is absent.
fn check_overwrite(outputs: &[&Path], force: bool) -> Result<(), SealError> {
    if force {
        return Ok(());
    }
    if let Some(existing) = outputs.iter().find(|p| p.exists()) {
        return Err(err(
            "output_exists",
            format!(
                "refusing to overwrite existing file {}; pass --force to replace",
                existing.display()
            ),
            2,
        ));
    }
    Ok(())
}

/// The final output paths: bundle, signer `.pub`, and (when attesting) `.operator.pub`.
fn output_paths(output: &Path, attesting: bool) -> (PathBuf, Option<PathBuf>) {
    let operator_pub = attesting.then(|| with_suffix(output, ".operator.pub"));
    (with_suffix(output, ".pub"), operator_pub)
}

/// Writes `contents` as a bundle to `path` (create/truncate).
fn write_bundle_file(path: &Path, bundle: &SessionBundle) -> Result<(), SealError> {
    let mut file = std::fs::File::create(path).map_err(|e| {
        err(
            "io_error",
            format!("cannot create temp file {}: {e}", path.display()),
            3,
        )
    })?;
    write_bundle(
        &mut file,
        &bundle.manifest,
        &bundle.events,
        &bundle.objects,
        &WriteBundleOptions::default(),
    )
    .map_err(|e| err("bundle_error", format!("bundle write failed: {e}"), 3))?;
    file.sync_all().map_err(|e| {
        err(
            "io_error",
            format!("cannot flush {}: {e}", path.display()),
            3,
        )
    })
}

/// Runs the whole seal pipeline. Prints nothing; the caller renders the outcome.
fn seal(args: &SealArgs) -> Result<SealOutcome, SealError> {
    // 1. Validate everything that needs no work: input kind, flags, keys, identity, overwrite.
    let input = classify_input(&args.input)?;
    if args.trace_id.is_some() && matches!(input, SealInput::Session(_)) {
        return Err(err(
            "usage",
            "--trace-id applies only when INPUT is a trace file".to_owned(),
            2,
        ));
    }
    if let Some(output) = &args.output {
        let (pub_path, op_pub) = output_paths(output, args.operator_key.is_some());
        let mut outs: Vec<&Path> = vec![output.as_path(), pub_path.as_path()];
        outs.extend(op_pub.as_deref());
        check_overwrite(&outs, args.force)?;
    }
    let signer = load_private_key(&args.key, "--key")?;
    let operator = match &args.operator_key {
        Some(path) => Some(load_private_key(path, "--operator-key")?),
        None => None,
    };
    let identity = operator_identity(args)?;

    // 2. Import the trace (or select the existing session).
    let (session_id, journal_dir, imported) = match input {
        SealInput::Trace(path) => {
            let options = ImportOptions {
                trace_id: args.trace_id.clone(),
                max_bytes: DEFAULT_MAX_TRACE_BYTES,
            };
            let outcome = import_trace_into_journal(&path, args.journal.clone(), &options)?;
            (outcome.report.session_id, outcome.journal_dir, true)
        }
        SealInput::Session(sid) => {
            let journal_dir = match args.journal.clone() {
                Some(dir) => dir,
                None => default_journal_dir().map_err(|e| {
                    err(
                        "journal_not_found",
                        format!("cannot resolve default journal directory: {e}"),
                        3,
                    )
                })?,
            };
            let present = journal_contains_session(&journal_dir, sid).map_err(|e| {
                err(
                    "journal_access",
                    format!("cannot read journal {}: {e}", journal_dir.display()),
                    3,
                )
            })?;
            if !present {
                return Err(err(
                    "session_not_found",
                    format!(
                        "session {sid} not found in journal {}",
                        journal_dir.display()
                    ),
                    2,
                ));
            }
            (sid, journal_dir, false)
        }
    };
    let sid = session_id.as_hyphenated().to_string();

    // 3. Resolve the outputs (the default name needs the session id) and re-check overwrite.
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{sid}.akmon")));
    let (pub_path, op_pub_path) = output_paths(&output, operator.is_some());
    {
        let mut outs: Vec<&Path> = vec![output.as_path(), pub_path.as_path()];
        outs.extend(op_pub_path.as_deref());
        check_overwrite(&outs, args.force)?;
    }

    // 4. Export the session into a temp bundle next to the output, then run the integrity gate
    //    `bundle sign` uses on the serialized bytes.
    let mut cleanup = Cleanup::default();
    let tmp_bundle = temp_path_for(&output, "bundle");
    cleanup.track(tmp_bundle.clone());
    let unsigned = assemble_session_bundle(session_id, &journal_dir)?;
    write_bundle_file(&tmp_bundle, &unsigned)?;
    drop(unsigned);
    let contents = read_bundle_verified_for_signing(&tmp_bundle)?;

    // 5. Sign the head, then (optionally) attest — the same helpers `bundle sign`/`attest` use.
    let mut sealed = SessionBundle {
        manifest: contents.manifest,
        events: contents.events,
        objects: contents.objects,
    };
    add_head_signature(&mut sealed.manifest, &signer.pkcs8)?;
    if let (Some(op), Some(identity)) = (&operator, &identity) {
        add_operator_attestation(
            &mut sealed.manifest,
            identity,
            &op.pkcs8,
            &format!(
                "--operator-key {}",
                args.operator_key
                    .as_deref()
                    .unwrap_or(Path::new(""))
                    .display()
            ),
        )?;
    }
    write_bundle_file(&tmp_bundle, &sealed)?;

    // 6. Write the public key file(s) exactly as `bundle keygen --public-out` does.
    let signer_hex = hex::encode(&signer.public_key);
    let tmp_pub = temp_path_for(&output, "pub");
    cleanup.track(tmp_pub.clone());
    write_public_key_file(&tmp_pub, &signer_hex, true).map_err(|e| {
        err(
            "io_error",
            format!("cannot write public key file {}: {e}", tmp_pub.display()),
            3,
        )
    })?;
    let tmp_op_pub = match &operator {
        Some(op) => {
            let tmp = temp_path_for(&output, "operator-pub");
            cleanup.track(tmp.clone());
            write_public_key_file(&tmp, &hex::encode(&op.public_key), true).map_err(|e| {
                err(
                    "io_error",
                    format!("cannot write public key file {}: {e}", tmp.display()),
                    3,
                )
            })?;
            Some(tmp)
        }
        None => None,
    };

    // 7. Self-verify with the written public key file(s), using the policy code of
    //    `bundle verify --verify-key <pub> --require-signature [--require-operator-key <op.pub>]`.
    let required_operator_keys: Vec<PathBuf> = tmp_op_pub.iter().cloned().collect();
    let policy = VerifyPolicy::load(
        std::slice::from_ref(&tmp_pub),
        true,
        &[],
        false,
        &required_operator_keys,
        false,
    )
    .map_err(|e| err(e.category, e.message, 3))?;
    let report = read_and_evaluate_bundle(
        &tmp_bundle,
        false,
        DEFAULT_MAX_BUNDLE_DECODED_BYTES,
        &policy,
    )?;
    if !report.passed {
        let detail = report
            .violations
            .first()
            .map(|v| format!("; first: [{}] {}", v.category, v.message))
            .unwrap_or_default();
        return Err(err(
            "self_verify_failed",
            format!(
                "the sealed bundle did not pass self-verification ({} violation(s){detail}); nothing was written",
                report.violations.len()
            ),
            1,
        ));
    }
    let capture_level = report.capture.as_ref().map_or_else(
        || CaptureLevel::Full.as_str().to_owned(),
        |c| c.level.clone(),
    );

    // 8. Optional prove-openssl artifacts (same writer as `bundle prove-openssl`).
    let mut openssl_commands = Vec::new();
    if let Some(dir) = &args.proof_dir {
        for name in proof_file_names(operator.is_some()) {
            cleanup.track(dir.join(name));
        }
        let emitted = emit_openssl_artifacts(
            &sealed.manifest,
            &signer.public_key,
            operator.as_ref().map(|op| op.public_key.as_slice()),
            dir,
            "--proof-dir",
        )?;
        openssl_commands.push(emitted.head.openssl_command);
        if let Some(op) = emitted.operator {
            openssl_commands.push(op.openssl_command);
        }
    }

    // 9. Everything verified: move the outputs into place.
    let mut renames = vec![(tmp_bundle, output.clone()), (tmp_pub, pub_path)];
    if let (Some(tmp), Some(fin)) = (tmp_op_pub, op_pub_path) {
        renames.push((tmp, fin));
    }
    for (tmp, fin) in renames {
        std::fs::rename(&tmp, &fin).map_err(|e| {
            err(
                "io_error",
                format!("cannot finalize {}: {e}", fin.display()),
                3,
            )
        })?;
        cleanup.track(fin);
    }
    cleanup.disarmed = true;

    Ok(SealOutcome {
        session_id: sid,
        output,
        signer: signer.public_key,
        operator: operator.map(|op| op.public_key),
        capture_level,
        proof_dir: args.proof_dir.clone(),
        openssl_commands,
        imported,
    })
}

/// The exact `agef-verify` hand-over command for the sealed outputs.
fn verify_command(outcome: &SealOutcome) -> String {
    let bundle = outcome.output.display().to_string();
    let (pub_path, op_pub) = output_paths(&outcome.output, outcome.operator.is_some());
    let mut cmd = format!(
        "agef-verify {} --verify-key {} --require-signature",
        shell_quote(&bundle),
        shell_quote(&pub_path.display().to_string())
    );
    if let Some(op) = op_pub {
        cmd.push_str(&format!(
            " --require-operator-key {}",
            shell_quote(&op.display().to_string())
        ));
    }
    cmd
}

/// Renders the human summary and hand-over instructions on stderr.
fn print_human(outcome: &SealOutcome, verify: &str) {
    let bundle_disp = bundle_export_output_display(&outcome.output);
    let (pub_path, op_pub) = output_paths(&outcome.output, outcome.operator.is_some());
    eprintln!("sealed: {bundle_disp}");
    eprintln!(
        "  session_id: {} ({})",
        sanitize_display(&outcome.session_id),
        if outcome.imported {
            "imported from OTEL trace"
        } else {
            "existing journal session"
        }
    );
    eprintln!("  capture level: {}", outcome.capture_level);
    match outcome.capture_level.as_str() {
        "structural" => eprintln!(
            "    note: metadata only; message content was NOT captured by the source telemetry"
        ),
        "partial" => {
            eprintln!("    note: some calls carry only metadata; content partially captured")
        }
        _ => {}
    }
    eprintln!("  integrity: OK | signature: verified | self-verify: passed");
    eprintln!("  signer key_id: {}", key_id(&outcome.signer));
    eprintln!(
        "  signer public key: {}",
        bundle_export_output_display(&pub_path)
    );
    if let (Some(op), Some(op_path)) = (&outcome.operator, &op_pub) {
        eprintln!("  operator key_id: {}", key_id(op));
        eprintln!(
            "  operator public key: {}",
            bundle_export_output_display(op_path)
        );
    }
    eprintln!();
    eprintln!("hand over the bundle; the auditor verifies offline with:");
    eprintln!("  {verify}");
    eprintln!("  {verify} --format markdown > report.md");
    if !outcome.openssl_commands.is_empty() {
        eprintln!("or with stock OpenSSL 3 only (artifacts in --proof-dir):");
        for cmd in &outcome.openssl_commands {
            eprintln!("  {cmd}");
        }
    }
    eprintln!(
        "send the public key file(s) over a separate trusted channel (not alongside the bundle);"
    );
    eprintln!("KEEP THE PRIVATE KEY SECRET.");
}

/// Runs `akmon seal`.
pub fn run_seal(args: &SealArgs) -> ExitCode {
    let json = matches!(args.format, BundleImportFormat::Json);
    let outcome = match seal(args) {
        Ok(outcome) => outcome,
        Err(e) => {
            if json {
                ToolErrorV1::new(TOOL, &e.category, e.message).print_json();
            } else {
                eprintln!("akmon: seal: {}", sanitize_display(&e.message));
            }
            return ExitCode::from(e.code);
        }
    };
    let verify = verify_command(&outcome);
    if json {
        let report = SealReportV1 {
            tool: TOOL.to_owned(),
            akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
            passed: true,
            session_id: outcome.session_id.clone(),
            bundle: bundle_export_output_display(&outcome.output),
            signer_public_key: hex::encode(&outcome.signer),
            signer_key_id: key_id(&outcome.signer),
            operator_public_key: outcome.operator.as_ref().map(hex::encode),
            capture_level: outcome.capture_level.clone(),
            proof_dir: outcome
                .proof_dir
                .as_ref()
                .map(|d| bundle_export_output_display(d)),
            verify_command: verify,
        };
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                ToolErrorV1::new(
                    TOOL,
                    "io_error",
                    format!("failed to render JSON output: {e}"),
                )
                .print_json();
                return ExitCode::from(3);
            }
        }
    } else {
        print_human(&outcome, &verify);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_is_appended_to_full_file_name() {
        assert_eq!(
            with_suffix(Path::new("out/audit.akmon"), ".pub"),
            PathBuf::from("out/audit.akmon.pub")
        );
        let (pub_path, op) = output_paths(Path::new("a.akmon"), true);
        assert_eq!(pub_path, PathBuf::from("a.akmon.pub"));
        assert_eq!(op, Some(PathBuf::from("a.akmon.operator.pub")));
    }

    #[test]
    fn temp_path_lives_next_to_output() {
        let tmp = temp_path_for(Path::new("dir/audit.akmon"), "bundle");
        assert_eq!(tmp.parent(), Some(Path::new("dir")));
        let bare = temp_path_for(Path::new("audit.akmon"), "pub");
        assert_eq!(bare.parent(), Some(Path::new(".")));
    }

    #[test]
    fn classify_rejects_missing_non_uuid_input() {
        let e = classify_input("/definitely/not/here.json")
            .err()
            .expect("refused");
        assert_eq!(e.code, 2);
        assert_eq!(e.category, "input_not_found");
        assert!(matches!(
            classify_input("4bf92f35-77b3-4da6-a3ce-929d0e0e4736"),
            Ok(SealInput::Session(_))
        ));
    }
}
