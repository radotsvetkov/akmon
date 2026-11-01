//! `akmon bundle prove-openssl` — emit the artifacts a third party needs to verify an Akmon
//! Ed25519 signature with stock `openssl` alone (no Akmon binary, no cloud).
//!
//! This is the reproducible proof of Akmon's offline-verifiability wedge (metric F.1). It is
//! READ-ONLY on the bundle: it reconstructs the canonical `AGEF-SIG-v1` statement via the locked
//! [`akmon_bundle::signing_statement`], extracts the already-present detached signature from
//! `manifest.signatures[]`, and re-encodes the supplied public key as SPKI PEM. It signs nothing
//! and writes nothing back into the bundle — it only writes side files into `--out-dir`.
//!
//! Honesty contract (TC-01): the openssl check only proves that the key holder signed session head
//! H. What ties H to the bundle's events and objects is the AGEF hash chain. So before writing ANY
//! artifact this command (1) runs full bundle integrity verification and (2) verifies the selected
//! signature (and, with `--operator-key`, the operator attestation) against the supplied key, and
//! refuses with exit 1 if either fails. The emitted artifact bytes are unchanged for valid bundles.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_bundle::report::ToolErrorV1;
use akmon_bundle::{
    Manifest, OPERATOR_STATEMENT_VERSION, SCHEME_ED25519, SIG_STATEMENT_VERSION, ed25519_spki_pem,
    key_id, operator_statement, parse_public_key_hex, sanitize_display, signing_statement,
    verify_statement,
};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::bundle_cmd::{
    BundleImportFormat, bundle_export_output_display, read_bundle_verified_for_signing,
};
use crate::otel_cmd::shell_quote;

/// File names of the three head-signature artifacts written into `--out-dir`.
const STATEMENT_FILE: &str = "statement.bin";
const SIGNATURE_FILE: &str = "signature.bin";
const PUBKEY_PEM_FILE: &str = "pubkey.pem";

/// File names of the three operator-attestation artifacts written into `--out-dir` when
/// `--operator-key` is supplied. Separate from the head-signature files so both proofs coexist.
const OPERATOR_STATEMENT_FILE: &str = "operator_statement.bin";
const OPERATOR_SIGNATURE_FILE: &str = "operator_signature.bin";
const OPERATOR_PUBKEY_PEM_FILE: &str = "operator_pubkey.pem";

/// Length in bytes of a raw Ed25519 detached signature.
const ED25519_SIGNATURE_LEN: usize = 64;

/// The one-sentence statement of what the emitted openssl command does and does not prove.
const OPENSSL_PROVES: &str = "the openssl command proves that the holder of this key signed session head H; \
the AGEF hash chain (checked by agef-verify or akmon bundle verify, and by this command before \
writing the artifacts) is what ties H to the bundle's events and objects";

/// Arguments for `akmon bundle prove-openssl`.
#[derive(Args, Debug, Clone)]
pub struct BundleProveArgs {
    /// Path to the signed `.akmon` bundle file.
    pub(crate) bundle: PathBuf,
    /// File containing the signer's raw Ed25519 public key as 64 hex characters — the same
    /// artifact `akmon bundle sign` prints and `akmon bundle verify --verify-key` consumes.
    #[arg(long = "verify-key", value_name = "HEX_FILE")]
    pub(crate) verify_key: PathBuf,
    /// Optional file containing an operator's raw Ed25519 public key as 64 hex characters — the
    /// public half of the key that produced an `akmon bundle attest` operator attestation. When
    /// supplied, the command additionally emits `operator_statement.bin`, `operator_signature.bin`,
    /// and `operator_pubkey.pem` so the operator attestation is verifiable with stock `openssl`
    /// too. Omitting it leaves the output byte-identical to a head-signature-only proof.
    #[arg(long = "operator-key", value_name = "HEX_FILE")]
    pub(crate) operator_key: Option<PathBuf>,
    /// Directory to write the three verification artifacts into. Defaults to the current directory.
    #[arg(long = "out-dir", value_name = "DIR", default_value = ".")]
    pub(crate) out_dir: PathBuf,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleImportFormat,
}

/// Stable JSON shape for `akmon bundle prove-openssl --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BundleProveReportV1 {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this report.
    akmon_version: String,
    /// Path to the bundle the artifacts were extracted from.
    bundle_path: String,
    /// Session UUID from the bundle manifest.
    session_id: String,
    /// Signer key id (lowercase hex SHA-256 of the public key) the signature was matched on.
    key_id: String,
    /// Path of the emitted statement bytes (the exact message that was signed).
    statement_path: String,
    /// Path of the emitted 64-byte raw detached signature.
    signature_path: String,
    /// Path of the emitted SPKI PEM public key.
    pubkey_pem_path: String,
    /// The exact `openssl` command a third party runs to verify, with real emitted paths.
    openssl_command: String,
    /// Operator-attestation artifacts. Present only when `--operator-key` was supplied; omitted
    /// entirely otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    operator: Option<OperatorProveFields>,
    /// Always `true` in a success report: full AGEF integrity verification passed before writing.
    #[serde(default)]
    integrity_verified: bool,
    /// Always `true` in a success report: the selected signature verified against `--verify-key`.
    #[serde(default)]
    signature_verified: bool,
    /// The session head H that the signature (and the openssl command) covers.
    #[serde(default)]
    head: String,
    /// What the openssl command proves, and what ties H to the bundle contents.
    #[serde(default)]
    openssl_proves: String,
}

/// The operator-attestation block of [`BundleProveReportV1`], emitted only with `--operator-key`.
///
/// `operator_id` and `role` are the operator's own self-asserted claims, reproduced verbatim from
/// the matched attestation; trust attaches to `key_id` (the verified key), never to the name.
#[derive(Debug, Serialize, Deserialize)]
pub struct OperatorProveFields {
    /// Operator key id (lowercase hex SHA-256 of the operator public key) the attestation matched.
    key_id: String,
    /// Self-asserted operator identifier from the matched attestation (not independently verified).
    operator_id: String,
    /// Self-asserted role from the matched attestation (not independently verified).
    role: String,
    /// Path of the emitted operator statement bytes (the exact `AGEF-OPERATOR-v1` message signed).
    statement_path: String,
    /// Path of the emitted 64-byte raw operator detached signature.
    signature_path: String,
    /// Path of the emitted operator SPKI PEM public key.
    pubkey_pem_path: String,
    /// The exact `openssl` command a third party runs to verify the operator attestation.
    openssl_command: String,
    /// Always `true` in a success report: the attestation verified against `--operator-key`.
    #[serde(default)]
    operator_key_verified: bool,
}

/// A refusal: `(category, message, exit_code)`.
pub(crate) type Refusal = (String, String, u8);

fn refusal(category: &str, msg: String, code: u8) -> Refusal {
    (category.to_owned(), msg, code)
}

/// One verified, ready-to-write proof: the statement bytes, raw signature, and SPKI PEM.
struct PreparedProof {
    key_id: String,
    statement: String,
    signature: Vec<u8>,
    pubkey_pem: String,
}

/// One artifact a verified proof consists of: its final path in the output directory and bytes.
pub(crate) struct ProofFile {
    /// Final path (`<out-dir>/<name>`, as the user gave the directory).
    pub(crate) path: PathBuf,
    /// Exact bytes to write (raw; no newline translation).
    pub(crate) bytes: Vec<u8>,
}

/// One verified proof laid out in the output directory, not yet written.
pub(crate) struct PlannedProof {
    /// Key id the signature / attestation was matched on.
    key_id: String,
    /// Statement, signature, and SPKI PEM files, in that order.
    files: [ProofFile; 3],
    /// The exact `openssl pkeyutl -verify` command, with shell-quoted paths as the user gave them.
    pub(crate) openssl_command: String,
}

/// The operator half of [`PlannedProofs`].
pub(crate) struct PlannedOperatorProof {
    proof: PlannedProof,
    operator_id: String,
    role: String,
}

/// Every verified proof requested, laid out in the output directory, not yet written. The caller
/// decides HOW to write the files (`prove-openssl` writes in place; `seal` stages temp files and
/// renames them into place together with the bundle).
pub(crate) struct PlannedProofs {
    /// The head-signature proof.
    pub(crate) head: PlannedProof,
    /// The operator-attestation proof, when requested.
    pub(crate) operator: Option<PlannedOperatorProof>,
}

impl PlannedProofs {
    /// All files to write, head proof first.
    pub(crate) fn files(&self) -> impl Iterator<Item = &ProofFile> {
        self.head
            .files
            .iter()
            .chain(self.operator.iter().flat_map(|op| op.proof.files.iter()))
    }

    /// The openssl commands, head proof first.
    pub(crate) fn openssl_commands(&self) -> Vec<String> {
        let mut cmds = vec![self.head.openssl_command.clone()];
        if let Some(op) = &self.operator {
            cmds.push(op.proof.openssl_command.clone());
        }
        cmds
    }
}

/// `<out_dir>/<name>` as it should appear in a printed command: the bare name when `out_dir` is
/// the current directory, otherwise the path as the user gave it (not canonicalized, so the
/// command stays meaningful on the auditor's machine relative to the handed-over directory).
fn command_path(out_dir: &Path, name: &str) -> String {
    if out_dir.as_os_str().is_empty() || out_dir == Path::new(".") {
        name.to_owned()
    } else {
        out_dir.join(name).display().to_string()
    }
}

/// The exact, copy-pasteable openssl verify command for one proof (every path shell-quoted).
fn openssl_command(out_dir: &Path, names: (&str, &str, &str)) -> String {
    let statement = shell_quote(&command_path(out_dir, names.0));
    let signature = shell_quote(&command_path(out_dir, names.1));
    let pubkey = shell_quote(&command_path(out_dir, names.2));
    format!(
        "openssl pkeyutl -verify -pubin -inkey {pubkey} -rawin -in {statement} -sigfile {signature}"
    )
}

/// Lays one verified proof out in `out_dir` under `names` (statement, signature, PEM).
fn plan_proof(out_dir: &Path, proof: PreparedProof, names: (&str, &str, &str)) -> PlannedProof {
    PlannedProof {
        openssl_command: openssl_command(out_dir, names),
        key_id: proof.key_id,
        files: [
            ProofFile {
                path: out_dir.join(names.0),
                bytes: proof.statement.into_bytes(),
            },
            ProofFile {
                path: out_dir.join(names.1),
                bytes: proof.signature,
            },
            ProofFile {
                path: out_dir.join(names.2),
                bytes: proof.pubkey_pem.into_bytes(),
            },
        ],
    }
}

/// Reads and parses a public-key hex file; `category`/`flag` label the error.
fn load_key(path: &Path, flag: &str, category: &str) -> Result<Vec<u8>, Refusal> {
    let hex_str = std::fs::read_to_string(path).map_err(|err| {
        refusal(
            category,
            format!("cannot read {flag} {}: {err}", path.display()),
            3,
        )
    })?;
    parse_public_key_hex(&hex_str)
        .map_err(|err| refusal(category, format!("{flag} {}: {err}", path.display()), 3))
}

/// Decodes a 64-byte Ed25519 signature from hex.
fn decode_signature(hex_sig: &str, what: &str, category: &str) -> Result<Vec<u8>, Refusal> {
    let bytes = hex::decode(hex_sig)
        .map_err(|err| refusal(category, format!("{what} is not valid hex: {err}"), 1))?;
    if bytes.len() != ED25519_SIGNATURE_LEN {
        return Err(refusal(
            category,
            format!(
                "{what} is {} bytes; expected {ED25519_SIGNATURE_LEN}",
                bytes.len()
            ),
            1,
        ));
    }
    Ok(bytes)
}

/// Selects and VERIFIES the head signature made by `pubkey`.
fn prepare_head_proof(manifest: &Manifest, pubkey: &[u8]) -> Result<PreparedProof, Refusal> {
    let kid = key_id(pubkey);
    let Some(signatures) = &manifest.signatures else {
        return Err(refusal(
            "no_matching_signature",
            format!("bundle has no signatures; cannot prove a signature for key_id {kid}"),
            1,
        ));
    };
    let Some(entry) = signatures.iter().find(|s| s.key_id == kid) else {
        return Err(refusal(
            "no_matching_signature",
            format!(
                "no manifest signature matches the supplied public key (expected key_id {kid})"
            ),
            1,
        ));
    };
    if entry.scheme != SCHEME_ED25519 || entry.statement_version != SIG_STATEMENT_VERSION {
        return Err(refusal(
            "unsupported_signature",
            format!(
                "signature for key_id {kid} is scheme={} statement_version={}; only ed25519/AGEF-SIG-v1 is supported",
                sanitize_display(&entry.scheme),
                sanitize_display(&entry.statement_version)
            ),
            1,
        ));
    }
    let signature = decode_signature(
        &entry.signature,
        &format!("signature for key_id {kid}"),
        "malformed_signature",
    )?;
    // Reconstruct the exact signed statement via the LOCKED library function, then check it.
    let statement = signing_statement(
        &manifest.agef_version,
        &manifest.hash_algorithm,
        &manifest.session.id,
        &manifest.session.head,
    );
    if verify_statement(statement.as_bytes(), &signature, pubkey).is_err() {
        return Err(refusal(
            "signature_invalid",
            format!(
                "the signature for key_id {kid} does NOT verify over this bundle's AGEF-SIG-v1 statement; refusing to emit artifacts"
            ),
            1,
        ));
    }
    let pubkey_pem = ed25519_spki_pem(pubkey).map_err(|err| {
        refusal(
            "verify_key_error",
            format!("cannot encode public key as PEM: {err}"),
            3,
        )
    })?;
    Ok(PreparedProof {
        key_id: kid,
        statement,
        signature,
        pubkey_pem,
    })
}

/// Selects and VERIFIES the operator attestation made by `operator_pubkey`. Returns the proof plus
/// the attestation's self-asserted `(operator_id, role)`.
fn prepare_operator_proof(
    manifest: &Manifest,
    operator_pubkey: &[u8],
) -> Result<(PreparedProof, String, String), Refusal> {
    let kid = key_id(operator_pubkey);
    let Some(attestations) = &manifest.operator_attestations else {
        return Err(refusal(
            "no_matching_operator_attestation",
            format!(
                "bundle has no operator attestations; cannot prove an attestation for key_id {kid}"
            ),
            1,
        ));
    };
    let Some(entry) = attestations.iter().find(|a| a.key_id == kid) else {
        return Err(refusal(
            "no_matching_operator_attestation",
            format!(
                "no operator attestation matches the supplied public key (expected key_id {kid})"
            ),
            1,
        ));
    };
    if entry.scheme != SCHEME_ED25519 || entry.statement_version != OPERATOR_STATEMENT_VERSION {
        return Err(refusal(
            "unsupported_operator_attestation",
            format!(
                "operator attestation for key_id {kid} is scheme={} statement_version={}; only ed25519/{OPERATOR_STATEMENT_VERSION} is supported",
                sanitize_display(&entry.scheme),
                sanitize_display(&entry.statement_version)
            ),
            1,
        ));
    }
    let signature = decode_signature(
        &entry.signature,
        &format!("operator attestation for key_id {kid}"),
        "malformed_operator_signature",
    )?;
    let statement = operator_statement(
        &manifest.agef_version,
        &manifest.hash_algorithm,
        &manifest.session.id,
        &manifest.session.head,
        &entry.operator_id,
        &entry.display_name,
        &entry.role,
        &entry.org,
    );
    if verify_statement(statement.as_bytes(), &signature, operator_pubkey).is_err() {
        return Err(refusal(
            "operator_signature_invalid",
            format!(
                "the operator attestation for key_id {kid} does NOT verify over this bundle's AGEF-OPERATOR-v1 statement; refusing to emit artifacts"
            ),
            1,
        ));
    }
    let pubkey_pem = ed25519_spki_pem(operator_pubkey).map_err(|err| {
        refusal(
            "operator_key_error",
            format!("cannot encode operator public key as PEM: {err}"),
            3,
        )
    })?;
    Ok((
        PreparedProof {
            key_id: kid,
            statement,
            signature,
            pubkey_pem,
        },
        entry.operator_id.clone(),
        entry.role.clone(),
    ))
}

/// File names [`plan_openssl_artifacts`] lays out in its output directory.
pub(crate) fn proof_file_names(with_operator: bool) -> Vec<&'static str> {
    let mut names = vec![STATEMENT_FILE, SIGNATURE_FILE, PUBKEY_PEM_FILE];
    if with_operator {
        names.extend([
            OPERATOR_STATEMENT_FILE,
            OPERATOR_SIGNATURE_FILE,
            OPERATOR_PUBKEY_PEM_FILE,
        ]);
    }
    names
}

/// Selects and VERIFIES the head signature (and, with `operator_pubkey`, the operator
/// attestation), then lays the openssl artifacts out in `out_dir` WITHOUT writing them. Shared by
/// `akmon bundle prove-openssl` and `akmon seal --proof-dir`, so both emit identical bytes.
/// The caller must already have run full integrity verification on the bundle.
pub(crate) fn plan_openssl_artifacts(
    manifest: &Manifest,
    pubkey: &[u8],
    operator_pubkey: Option<&[u8]>,
    out_dir: &Path,
) -> Result<PlannedProofs, Refusal> {
    let head_proof = prepare_head_proof(manifest, pubkey)?;
    let operator_proof = match operator_pubkey {
        Some(key) => Some(prepare_operator_proof(manifest, key)?),
        None => None,
    };
    Ok(plan_verified_proofs(out_dir, head_proof, operator_proof))
}

/// Lays already-verified proofs out in `out_dir`.
fn plan_verified_proofs(
    out_dir: &Path,
    head_proof: PreparedProof,
    operator_proof: Option<(PreparedProof, String, String)>,
) -> PlannedProofs {
    PlannedProofs {
        head: plan_proof(
            out_dir,
            head_proof,
            (STATEMENT_FILE, SIGNATURE_FILE, PUBKEY_PEM_FILE),
        ),
        operator: operator_proof.map(|(proof, operator_id, role)| PlannedOperatorProof {
            proof: plan_proof(
                out_dir,
                proof,
                (
                    OPERATOR_STATEMENT_FILE,
                    OPERATOR_SIGNATURE_FILE,
                    OPERATOR_PUBKEY_PEM_FILE,
                ),
            ),
            operator_id,
            role,
        }),
    }
}

/// Writes every planned file in place into `out_dir` (created if missing).
fn write_planned_proofs(
    out_dir: &Path,
    planned: &PlannedProofs,
    out_dir_flag: &str,
) -> Result<(), Refusal> {
    std::fs::create_dir_all(out_dir).map_err(|err| {
        refusal(
            "io_error",
            format!("cannot create {out_dir_flag} {}: {err}", out_dir.display()),
            3,
        )
    })?;
    for file in planned.files() {
        std::fs::write(&file.path, &file.bytes).map_err(|err| {
            refusal(
                "io_error",
                format!("cannot write {}: {err}", file.path.display()),
                3,
            )
        })?;
    }
    Ok(())
}

/// Verifies the bundle and the signature(s), then emits the openssl verification artifacts.
pub fn run_bundle_prove_openssl(args: &BundleProveArgs) -> ExitCode {
    match prove(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err((category, msg, code)) => {
            if matches!(args.format, BundleImportFormat::Json) {
                ToolErrorV1::new("akmon", &category, msg).print_json();
            } else {
                eprintln!("akmon: bundle prove-openssl: {}", sanitize_display(&msg));
            }
            ExitCode::from(code)
        }
    }
}

fn prove(args: &BundleProveArgs) -> Result<(), Refusal> {
    // 1. Read the bundle and run FULL integrity verification (refuse on any violation).
    let contents = read_bundle_verified_for_signing(&args.bundle)?;
    let manifest = &contents.manifest;

    // 2. Load keys, then select and VERIFY every requested proof before writing anything.
    let pubkey = load_key(&args.verify_key, "--verify-key", "verify_key_error")?;
    let head_proof = prepare_head_proof(manifest, &pubkey)?;
    let operator_proof = match &args.operator_key {
        Some(path) => {
            let operator_pubkey = load_key(path, "--operator-key", "operator_key_error")?;
            Some(prepare_operator_proof(manifest, &operator_pubkey)?)
        }
        None => None,
    };

    // 3. Everything verified: write the artifacts into --out-dir.
    let planned = plan_verified_proofs(&args.out_dir, head_proof, operator_proof);
    write_planned_proofs(&args.out_dir, &planned, "--out-dir")?;
    let PlannedProofs { head, operator } = planned;
    let [statement, signature, pubkey_pem] = &head.files;
    let operator = operator.map(|op| {
        let [op_statement, op_signature, op_pubkey_pem] = &op.proof.files;
        OperatorProveFields {
            key_id: op.proof.key_id.clone(),
            operator_id: op.operator_id,
            role: op.role,
            statement_path: bundle_export_output_display(&op_statement.path),
            signature_path: bundle_export_output_display(&op_signature.path),
            pubkey_pem_path: bundle_export_output_display(&op_pubkey_pem.path),
            openssl_command: op.proof.openssl_command.clone(),
            operator_key_verified: true,
        }
    });

    let report = BundleProveReportV1 {
        tool: "akmon".to_owned(),
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        bundle_path: bundle_export_output_display(args.bundle.as_path()),
        session_id: manifest.session.id.clone(),
        key_id: head.key_id.clone(),
        statement_path: bundle_export_output_display(&statement.path),
        signature_path: bundle_export_output_display(&signature.path),
        pubkey_pem_path: bundle_export_output_display(&pubkey_pem.path),
        openssl_command: head.openssl_command.clone(),
        operator,
        integrity_verified: true,
        signature_verified: true,
        head: manifest.session.head.clone(),
        openssl_proves: OPENSSL_PROVES.to_owned(),
    };
    match args.format {
        BundleImportFormat::Json => {
            let json = serde_json::to_string_pretty(&report).map_err(|err| {
                refusal(
                    "io_error",
                    format!("failed to render JSON output: {err}"),
                    3,
                )
            })?;
            println!("{json}");
        }
        BundleImportFormat::Human => print_human(&report),
    }
    Ok(())
}

fn print_human(report: &BundleProveReportV1) {
    eprintln!(
        "verified, then wrote openssl verification artifacts for: {}",
        report.bundle_path
    );
    eprintln!("  integrity: OK (full AGEF verification passed)");
    eprintln!(
        "  signature: verified against the supplied key (key_id: {})",
        report.key_id
    );
    eprintln!("  session_id: {}", sanitize_display(&report.session_id));
    eprintln!("  head: {}", sanitize_display(&report.head));
    eprintln!("  statement: {}", report.statement_path);
    eprintln!("  signature: {}", report.signature_path);
    eprintln!("  public key (PEM): {}", report.pubkey_pem_path);
    eprintln!("  verify offline with OpenSSL 3.x (no Akmon binary):");
    eprintln!("    {}", report.openssl_command);
    eprintln!("  what this proves: the holder of this key signed session head H (above).");
    eprintln!(
        "  what ties H to the bundle contents: the AGEF hash chain — check it independently with `agef-verify`."
    );
    if let Some(op) = &report.operator {
        eprintln!("operator attestation artifacts:");
        eprintln!("  operator key_id (verified): {}", op.key_id);
        eprintln!(
            "  operator_id (self-asserted): {}",
            sanitize_display(&op.operator_id)
        );
        eprintln!("  role (self-asserted): {}", sanitize_display(&op.role));
        eprintln!("  operator statement: {}", op.statement_path);
        eprintln!("  operator signature: {}", op.signature_path);
        eprintln!("  operator public key (PEM): {}", op.pubkey_pem_path);
        eprintln!("  verify the operator attestation offline with OpenSSL 3.x:");
        eprintln!("    {}", op.openssl_command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openssl_command_uses_bare_names_for_current_dir() {
        assert_eq!(
            openssl_command(
                Path::new("."),
                (STATEMENT_FILE, SIGNATURE_FILE, PUBKEY_PEM_FILE)
            ),
            "openssl pkeyutl -verify -pubin -inkey pubkey.pem -rawin -in statement.bin -sigfile signature.bin"
        );
    }

    #[test]
    fn openssl_command_shell_quotes_paths_as_given() {
        assert_eq!(
            openssl_command(
                Path::new("proof dir"),
                (STATEMENT_FILE, SIGNATURE_FILE, PUBKEY_PEM_FILE)
            ),
            "openssl pkeyutl -verify -pubin -inkey 'proof dir/pubkey.pem' -rawin -in 'proof dir/statement.bin' -sigfile 'proof dir/signature.bin'"
        );
        assert_eq!(
            openssl_command(
                Path::new("proof"),
                (STATEMENT_FILE, SIGNATURE_FILE, PUBKEY_PEM_FILE)
            ),
            "openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin -in proof/statement.bin -sigfile proof/signature.bin"
        );
    }
}
