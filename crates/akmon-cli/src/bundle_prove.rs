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

use std::path::PathBuf;
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
    pub(crate) openssl_command: String,
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

/// Paths of the three files a [`PreparedProof`] was written to, plus the openssl command.
pub(crate) struct WrittenProof {
    statement_path: String,
    signature_path: String,
    pubkey_pem_path: String,
    /// The exact `openssl pkeyutl -verify` command with the real emitted paths.
    pub(crate) openssl_command: String,
}

/// Reads and parses a public-key hex file; `category`/`flag` label the error.
fn load_key(path: &std::path::Path, flag: &str, category: &str) -> Result<Vec<u8>, Refusal> {
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

/// Writes one proof's three files (raw bytes; no newline translation) and builds its command.
fn write_proof(
    out_dir: &std::path::Path,
    proof: &PreparedProof,
    names: (&str, &str, &str),
) -> Result<WrittenProof, Refusal> {
    let statement_path = out_dir.join(names.0);
    let signature_path = out_dir.join(names.1);
    let pubkey_pem_path = out_dir.join(names.2);
    let writes: [(&std::path::Path, &[u8]); 3] = [
        (&statement_path, proof.statement.as_bytes()),
        (&signature_path, &proof.signature),
        (&pubkey_pem_path, proof.pubkey_pem.as_bytes()),
    ];
    for (path, bytes) in writes {
        std::fs::write(path, bytes).map_err(|err| {
            refusal(
                "io_error",
                format!("cannot write {}: {err}", path.display()),
                3,
            )
        })?;
    }
    let pubkey_pem_disp = bundle_export_output_display(pubkey_pem_path.as_path());
    let statement_disp = bundle_export_output_display(statement_path.as_path());
    let signature_disp = bundle_export_output_display(signature_path.as_path());
    let openssl_command = format!(
        "openssl pkeyutl -verify -pubin -inkey {pubkey_pem_disp} -rawin -in {statement_disp} -sigfile {signature_disp}"
    );
    Ok(WrittenProof {
        statement_path: statement_disp,
        signature_path: signature_disp,
        pubkey_pem_path: pubkey_pem_disp,
        openssl_command,
    })
}

/// The artifacts [`write_verified_proofs`] wrote, ready for a report.
pub(crate) struct EmittedProofs {
    /// Signer key id the head signature was matched on.
    pub(crate) head_key_id: String,
    /// The three head-signature files plus the openssl command.
    pub(crate) head: WrittenProof,
    /// The operator-attestation block, when an operator proof was requested.
    pub(crate) operator: Option<OperatorProveFields>,
}

/// File names [`write_verified_proofs`] may create in its output directory.
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
/// attestation), then writes the openssl artifacts into `out_dir`. Shared by
/// `akmon bundle prove-openssl` and `akmon seal --proof-dir`, so both emit identical bytes.
/// The caller must already have run full integrity verification on the bundle.
pub(crate) fn emit_openssl_artifacts(
    manifest: &Manifest,
    pubkey: &[u8],
    operator_pubkey: Option<&[u8]>,
    out_dir: &std::path::Path,
    out_dir_flag: &str,
) -> Result<EmittedProofs, Refusal> {
    let head_proof = prepare_head_proof(manifest, pubkey)?;
    let operator_proof = match operator_pubkey {
        Some(key) => Some(prepare_operator_proof(manifest, key)?),
        None => None,
    };
    write_verified_proofs(out_dir, head_proof, operator_proof, out_dir_flag)
}

/// Writes already-verified proofs into `out_dir` (created if missing).
fn write_verified_proofs(
    out_dir: &std::path::Path,
    head_proof: PreparedProof,
    operator_proof: Option<(PreparedProof, String, String)>,
    out_dir_flag: &str,
) -> Result<EmittedProofs, Refusal> {
    std::fs::create_dir_all(out_dir).map_err(|err| {
        refusal(
            "io_error",
            format!("cannot create {out_dir_flag} {}: {err}", out_dir.display()),
            3,
        )
    })?;
    let head = write_proof(
        out_dir,
        &head_proof,
        (STATEMENT_FILE, SIGNATURE_FILE, PUBKEY_PEM_FILE),
    )?;
    let operator = match operator_proof {
        Some((proof, operator_id, role)) => {
            let written = write_proof(
                out_dir,
                &proof,
                (
                    OPERATOR_STATEMENT_FILE,
                    OPERATOR_SIGNATURE_FILE,
                    OPERATOR_PUBKEY_PEM_FILE,
                ),
            )?;
            Some(OperatorProveFields {
                key_id: proof.key_id,
                operator_id,
                role,
                statement_path: written.statement_path,
                signature_path: written.signature_path,
                pubkey_pem_path: written.pubkey_pem_path,
                openssl_command: written.openssl_command,
                operator_key_verified: true,
            })
        }
        None => None,
    };
    Ok(EmittedProofs {
        head_key_id: head_proof.key_id,
        head,
        operator,
    })
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
    let EmittedProofs {
        head_key_id,
        head: head_written,
        operator,
    } = write_verified_proofs(&args.out_dir, head_proof, operator_proof, "--out-dir")?;

    let report = BundleProveReportV1 {
        tool: "akmon".to_owned(),
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        bundle_path: bundle_export_output_display(args.bundle.as_path()),
        session_id: manifest.session.id.clone(),
        key_id: head_key_id,
        statement_path: head_written.statement_path,
        signature_path: head_written.signature_path,
        pubkey_pem_path: head_written.pubkey_pem_path,
        openssl_command: head_written.openssl_command,
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
