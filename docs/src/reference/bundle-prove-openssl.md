# akmon bundle prove-openssl

Documented for Akmon `2.1.0`.

## Who this is for

Auditors, regulators, and counterparties who want to verify an Akmon bundle's Ed25519 signature
**with stock `openssl` alone** — no Akmon binary, no cloud, no lock-in. This is the reproducible
proof of Akmon's offline-verifiability guarantee (metric F.1): the signature format is a standard
PureEd25519 detached signature over a canonical ASCII statement, so any third party can check it.

## What you will have at the end

Three files in `--out-dir` plus a copy-paste `openssl` command:

- `statement.bin` — the exact `AGEF-SIG-v1` bytes that were signed (the session-head commitment).
- `signature.bin` — the 64-byte raw detached Ed25519 signature, extracted from the bundle manifest.
- `pubkey.pem` — the signer's public key in SPKI PEM form (the form `openssl` can ingest).

The command signs nothing and never modifies the bundle. It reads the bundle, reconstructs the
signed statement, extracts the matching signature, and re-encodes the supplied public key.

## Prerequisites

- A signed `.akmon` bundle (see [akmon sign](./sign.md) / produced by `akmon bundle sign`).
- The signer's public key as 64 hex characters in a file — the same artifact `akmon bundle sign`
  prints and `akmon bundle verify --verify-key` consumes.
- **OpenSSL 3.x** for the verification step. Stock LibreSSL (the macOS `/usr/bin/openssl`) cannot
  verify Ed25519 — it lacks `-rawin` and cannot load Ed25519 keys. Use an OpenSSL 3.x build.

## Steps

Emit the artifacts:

```bash
akmon bundle prove-openssl /path/to/audit.akmon --verify-key signer.pub.hex --out-dir ./proof
```

Then verify offline with OpenSSL 3.x (the command is also printed by the step above):

```bash
openssl pkeyutl -verify -pubin -inkey ./proof/pubkey.pem -rawin -in ./proof/statement.bin -sigfile ./proof/signature.bin
```

A valid signature prints `Signature Verified Successfully` and exits `0`. Tampering with
`statement.bin` (or using the wrong signature) makes `openssl` print `Signature Verification
Failure` and exit non-zero.

Optional flags:

- `--out-dir <DIR>` — directory for the three artifacts (default: current directory).
- `--format human|json` — default `human`. JSON emits **BundleProveReportV1** with the artifact
  paths and the exact `openssl_command`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Artifacts written; the printed `openssl` command is ready to run |
| `1` | No signature matches the supplied key, or the matched signature is unsupported/malformed |
| `3` | I/O or environment error (bundle or `--verify-key` unreadable, malformed archive, out-dir not writable) |

## How this proves the wedge

The signature in `manifest.signatures[]` is a PureEd25519 signature over the deterministic
`AGEF-SIG-v1` statement (version tag, AGEF version, hash algorithm, session id, and head, each on
its own LF-terminated line). `prove-openssl` writes those exact bytes verbatim and the raw 64-byte
signature, and wraps the public key in the RFC 8410 SPKI encoding `openssl` expects. Nothing about
the verification depends on Akmon — only on `openssl` and the public key the verifier already
trusts.

## See also

- [akmon bundle verify](./bundle-verify.md)
- [akmon sign](./sign.md)
- [agef-verify](./agef-verify.md)
