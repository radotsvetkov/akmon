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

With the optional `--operator-key` flag it additionally emits three operator-attestation artifacts
(`operator_statement.bin`, `operator_signature.bin`, `operator_pubkey.pem`) so the bundle's
`AGEF-OPERATOR-v1` operator attestation is verifiable with stock `openssl` too. See the
[`--operator-key`](#operator-attestation-with---operator-key) section below.

## Prerequisites

- A signed `.akmon` bundle produced by `akmon bundle sign`. The signing key is made with
  [`akmon bundle keygen`](./bundle-keygen.md) — note that `openssl genpkey` emits PKCS#8 v1, which
  `ring` rejects, so it is **not** a substitute for `keygen`.
- The signer's public key as 64 hex characters in a file — the same artifact `akmon bundle keygen`
  (`--public-out`) / `akmon bundle sign` produces and `akmon bundle verify --verify-key` consumes.
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

- `--operator-key <HEX_FILE>` — also emit the operator-attestation artifacts (see below).
- `--out-dir <DIR>` — directory for the artifacts (default: current directory).
- `--format human|json` — default `human`. JSON emits **BundleProveReportV1** with the artifact
  paths and the exact `openssl_command` (plus an `operator` block when `--operator-key` is given).

## Operator attestation with `--operator-key`

`--operator-key` is **optional and additive**: without it, the output (the three files, the human
text, and the JSON) is exactly as above. With it, the command **additionally** reads the operator's
raw Ed25519 public key (64 hex characters — the public half of the key that made an
[`akmon bundle attest`](./bundle-attest.md) operator attestation), finds the matching attestation in
`manifest.operator_attestations[]`, and emits three more files into `--out-dir`:

- `operator_statement.bin` — the exact `AGEF-OPERATOR-v1` bytes that were signed (the session head
  bound to the four self-asserted operator identity fields).
- `operator_signature.bin` — the 64-byte raw detached Ed25519 signature from the attestation.
- `operator_pubkey.pem` — the operator's public key in SPKI PEM form.

The `--verify-key` (head signature) artifacts are unchanged; the operator files sit alongside them.
The JSON gains an `operator` block with `key_id`, the **self-asserted** `operator_id` and `role`,
the three artifact paths, and the operator `openssl_command`. As with attestation, what `openssl`
proves is that the holder of the operator key signed those fields — trust in the **name** is
out-of-band (see [`akmon bundle attest`](./bundle-attest.md)).

```bash
akmon bundle prove-openssl /path/to/audit.akmon \
  --verify-key signer.pub.hex --operator-key operator.pub.hex --out-dir ./proof
```

Verify the operator attestation offline with OpenSSL 3.x (also printed by the step above):

```bash
openssl pkeyutl -verify -pubin -inkey ./proof/operator_pubkey.pem -rawin -in ./proof/operator_statement.bin -sigfile ./proof/operator_signature.bin
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Artifacts written; the printed `openssl` command(s) are ready to run |
| `1` | No signature matches `--verify-key` (unsupported/malformed); or `--operator-key` has no matching/unsupported/malformed operator attestation |
| `3` | I/O or environment error (bundle, `--verify-key`, or `--operator-key` unreadable, malformed archive, out-dir not writable) |

## How this proves the wedge

The signature in `manifest.signatures[]` is a PureEd25519 signature over the deterministic
`AGEF-SIG-v1` statement (version tag, AGEF version, hash algorithm, session id, and head, each on
its own LF-terminated line). `prove-openssl` writes those exact bytes verbatim and the raw 64-byte
signature, and wraps the public key in the RFC 8410 SPKI encoding `openssl` expects. Nothing about
the verification depends on Akmon — only on `openssl` and the public key the verifier already
trusts.

## See also

- [akmon bundle keygen](./bundle-keygen.md)
- [akmon bundle attest](./bundle-attest.md)
- [akmon bundle verify](./bundle-verify.md)
- [akmon sign](./sign.md)
- [agef-verify](./agef-verify.md)
