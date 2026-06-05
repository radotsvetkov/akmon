//! `akmon bundle keygen` — generate an Ed25519 signing key for `akmon bundle sign`.
//!
//! This closes the "can sign but cannot make a key" gap: `akmon bundle sign --key` consumes RAW
//! PKCS#8 **v2** DER bytes (the exact form [`akmon_bundle::generate_pkcs8`] emits and `ring`'s
//! `Ed25519KeyPair::from_pkcs8` accepts), which `openssl genpkey` cannot produce (it emits PKCS#8
//! v1, which `ring` rejects). This command writes those raw DER bytes to `--out` and surfaces the
//! public key (hex) + key_id so the signer can immediately distribute the public half.
//!
//! It touches NO bundle, manifest, events, objects, hash chain, or signature scheme — it only calls
//! the three pure key helpers ([`generate_pkcs8`], [`public_key_from_pkcs8`], [`key_id`]) and writes
//! two side files: the private key (0600 on unix, set at create time) and an optional public-key hex.

use std::path::PathBuf;
use std::process::ExitCode;

use akmon_bundle::{generate_pkcs8, key_id, public_key_from_pkcs8};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::bundle_cmd::{BundleImportFormat, bundle_export_output_display};

/// Arguments for `akmon bundle keygen`.
#[derive(Args, Debug, Clone)]
pub struct BundleKeygenArgs {
    /// Destination for the PKCS#8 v2 DER private key (raw bytes; the exact form
    /// `akmon bundle sign --key` consumes). Created with `0600` perms on unix.
    #[arg(long = "out", value_name = "FILE")]
    pub(crate) out: PathBuf,
    /// Also write the public key as 64 hex characters here (ready for
    /// `akmon bundle verify --verify-key` / `akmon bundle prove-openssl --verify-key`).
    #[arg(long = "public-out", value_name = "FILE")]
    pub(crate) public_out: Option<PathBuf>,
    /// Allow overwriting an existing `--out` (and `--public-out`) file. Off by default: keygen
    /// refuses to clobber an existing private key.
    #[arg(long)]
    pub(crate) force: bool,
    /// Output format for status messages: human (default) or json.
    #[arg(long, default_value = "human")]
    pub(crate) format: BundleImportFormat,
}

/// Stable JSON shape for `akmon bundle keygen --format json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct KeygenReportV1 {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this report.
    akmon_version: String,
    /// Path the PKCS#8 v2 DER private key was written to.
    key_path: String,
    /// Path the public-key hex was written to, or `null` when `--public-out` was not given.
    public_out: Option<String>,
    /// The raw Ed25519 public key as 64 lowercase hex characters (safe to distribute).
    public_key_hex: String,
    /// Signer key id: lowercase hex SHA-256 of the public key (matches `manifest.signatures[].key_id`).
    key_id: String,
}

/// Generates a fresh Ed25519 key, writes it to `--out` (0600 on unix), and surfaces the public key.
pub fn run_bundle_keygen(args: &BundleKeygenArgs) -> ExitCode {
    let json = matches!(args.format, BundleImportFormat::Json);
    let fail = |category: &str, msg: String, code: u8| -> ExitCode {
        if json {
            let _ = print_keygen_json_error(category, msg);
        } else {
            eprintln!("akmon: bundle keygen: {msg}");
        }
        ExitCode::from(code)
    };

    // 1. Clobber-precheck --public-out BEFORE generating or writing anything, so a refusal never
    //    leaves a half-written --out behind. (--out is protected atomically by create_new below.)
    if let Some(public_out) = &args.public_out
        && !args.force
        && public_out.exists()
    {
        return fail(
            "output_exists",
            format!(
                "refusing to overwrite existing file {}; pass --force to replace",
                public_out.display()
            ),
            3,
        );
    }

    // 2. Generate the key in memory. If this fails, nothing is written.
    let pkcs8 = match generate_pkcs8() {
        Ok(bytes) => bytes,
        Err(err) => return fail("keygen_failed", format!("key generation failed: {err}"), 3),
    };
    let pubkey = match public_key_from_pkcs8(&pkcs8) {
        Ok(pk) => pk,
        Err(err) => return fail("keygen_failed", format!("key generation failed: {err}"), 3),
    };
    let public_key_hex = hex::encode(&pubkey);
    let key_id = key_id(&pubkey);

    // 3. Open + write --out. create_new(true) (O_CREAT|O_EXCL) gives atomic, TOCTOU-free
    //    no-clobber; --force uses create+truncate. On unix the file is created 0600 and we
    //    re-assert 0600 on the open fd (closes the --force-over-wide-perms gap, race-free since
    //    contents are not written yet).
    let mut file = match create_private_key_file(&args.out, args.force) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return fail(
                "output_exists",
                format!(
                    "refusing to overwrite existing key {}; pass --force to replace",
                    args.out.display()
                ),
                3,
            );
        }
        Err(err) => {
            return fail(
                "io_error",
                format!("cannot create key file {}: {err}", args.out.display()),
                3,
            );
        }
    };
    use std::io::Write;
    if let Err(err) = file.write_all(&pkcs8).and_then(|()| file.flush()) {
        return fail(
            "io_error",
            format!("cannot write key file {}: {err}", args.out.display()),
            3,
        );
    }
    drop(file);

    // 4. Write --public-out (NOT secret): exactly 64 hex chars, no trailing newline. Use
    //    create_new (atomic no-clobber) unless --force.
    if let Some(public_out) = &args.public_out
        && let Err(err) = write_public_key_file(public_out, &public_key_hex, args.force)
    {
        return fail(
            "io_error",
            format!(
                "cannot write public key file {}: {err}",
                public_out.display()
            ),
            3,
        );
    }

    // 5. Emit output (files now exist, so display paths can be canonicalized).
    let key_path = bundle_export_output_display(args.out.as_path());
    let public_out_disp = args
        .public_out
        .as_ref()
        .map(|p| bundle_export_output_display(p.as_path()));

    match args.format {
        BundleImportFormat::Json => {
            let report = KeygenReportV1 {
                tool: "akmon".to_owned(),
                akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
                key_path,
                public_out: public_out_disp,
                public_key_hex,
                key_id,
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
            eprintln!("wrote ed25519 private key (PKCS#8 v2 DER): {key_path}");
            eprintln!("  key_id: {key_id}");
            eprintln!("  public key (hex): {public_key_hex}");
            if let Some(disp) = &public_out_disp {
                eprintln!("  public key written to: {disp}");
            }
            eprintln!(
                "  KEEP THE PRIVATE KEY SECRET. Distribute only the public key (hex) to verifiers."
            );
            eprintln!(
                "  verify a signed bundle with: akmon bundle verify <bundle> --verify-key <public-key-hex-file>"
            );
        }
    }
    ExitCode::SUCCESS
}

/// Opens `--out` for the private key. No-clobber via `create_new` unless `force`. On unix the file
/// is created `0600` (via `mode` on the open) and `0600` is re-asserted on the returned fd before
/// any bytes are written — this covers a `--force` overwrite of a pre-existing wide-perms inode
/// with no race (we already hold the fd; contents are written by the caller afterwards).
fn create_private_key_file(path: &std::path::Path, force: bool) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// Writes the public-key hex to `path` (exactly the 64 hex chars, no trailing newline). The public
/// key is not secret, so no special perms; `create_new` gives atomic no-clobber unless `force`.
fn write_public_key_file(
    path: &std::path::Path,
    public_key_hex: &str,
    force: bool,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    let mut file = opts.open(path)?;
    file.write_all(public_key_hex.as_bytes())?;
    file.flush()
}

/// JSON shape emitted when `keygen` cannot complete (identical contract to the sibling commands).
#[derive(Debug, Serialize, Deserialize)]
struct KeygenError {
    /// Producer tool name.
    tool: String,
    /// Akmon crate version that produced this error object.
    akmon_version: String,
    /// Human-readable error description.
    error: String,
    /// Stable error category for automation.
    category: String,
}

fn print_keygen_json_error(category: &str, error: String) -> std::io::Result<()> {
    let body = KeygenError {
        tool: "akmon".to_owned(),
        akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
        error,
        category: category.to_owned(),
    };
    let json =
        serde_json::to_string_pretty(&body).map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("{json}");
    Ok(())
}
