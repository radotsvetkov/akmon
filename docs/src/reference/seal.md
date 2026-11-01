# akmon seal

Documented for Akmon `3.0.0`.

Seal a session into one signed, self-verified `.akmon` bundle in a single command. `akmon seal` runs the whole evidence chain that otherwise takes four or five commands, and refuses to report success unless the result verifies.

## Usage

```text
akmon seal <INPUT> --key <signer.pk8>
           [--output <PATH>] [--journal <DIR>] [--trace-id <HEX>]
           [--operator-key <op.pk8> --operator-id <ID>
             [--display-name <S>] [--role <S>] [--org <S>]]
           [--proof-dir <DIR>] [--force] [--format human|json]
```

`INPUT` is either:

- an **OTLP/JSON trace file**, imported through the [OpenTelemetry importer](./otel-import.md) with its capture level (`full`, `partial`, or `structural`) recorded honestly, or
- an existing **journal session id** (a UUID), for example a session recorded by the bundled reference agent.

## What it does

1. Imports the trace into the journal, or selects the existing session.
2. Exports the session as an AGEF bundle.
3. Verifies the bundle's integrity.
4. Signs the session head with `--key` (Ed25519, `AGEF-SIG-v1`).
5. With `--operator-key`, adds an operator attestation (`AGEF-OPERATOR-v1`).
6. Verifies the bundle again with the public key(s) derived from the private key(s) it just used.
7. With `--proof-dir`, writes the [OpenSSL proof artifacts](./bundle-prove-openssl.md).

Before step 1, `akmon seal` checks that the output's directory exists and is writable, that `--proof-dir` exists or can be created, and that no output it would write already exists (unless `--force`). So the common failures happen before a trace is imported. `akmon seal` does **not** create the output's parent directory; create it first (`mkdir -p evidence`). A successful trace import is the one step that persists: if a later step fails, the imported session stays in the journal, and a retry imports the trace again.

## Outputs

| File | Content |
| --- | --- |
| `<OUTPUT>` | The sealed bundle. Defaults to `./<session-id>.akmon`. Written atomically, and nothing is left behind on failure; an existing file is not overwritten without `--force`. |
| `<OUTPUT>.pub` | The signer's public key as 64 hex characters, the same format as `akmon bundle keygen --public-out`. |
| `<OUTPUT>.operator.pub` | The operator's public key, only when attesting. |
| `<proof-dir>/statement.bin`, `signature.bin`, `pubkey.pem` | OpenSSL proof artifacts, only with `--proof-dir` (plus the `operator_*` files when attesting). The directory is created if missing; existing artifacts in it are not replaced without `--force`. |

Every file is written to a temp file and renamed into place only after self-verification passes. On failure nothing is left behind, and seal never removes a file it did not create.

The command prints the exact hand-over commands, for example:

```text
agef-verify audit.akmon --verify-key audit.akmon.pub --require-signature
agef-verify audit.akmon --verify-key audit.akmon.pub --require-signature --format markdown > report.md
```

When attesting, the printed commands also pass `--require-operator-key <OUTPUT>.operator.pub`. With `--proof-dir` it adds the `openssl` command(s), and it always ends with a reminder to send the public key file(s) over a separate trusted channel.

The `.pub` files are a convenience: they are what you **send** to the verifier over that separate channel. The printed commands name them by path so they are easy to copy, but the verifier should run them with the key they received out-of-band, not with a `.pub` file that arrived in the same package as the bundle or is stored next to it. Anyone who can replace the bundle can replace a key stored beside it.

## Options

| Option | Meaning |
| --- | --- |
| `--key <FILE>` | Signer private key, PKCS#8 v2 DER from [`akmon bundle keygen`](./bundle-keygen.md). Required. |
| `--output <PATH>` | Bundle path. Default `./<session-id>.akmon`. |
| `--journal <DIR>` | Journal directory to import into or read from. Default: the per-user journal. |
| `--trace-id <HEX>` | Select one trace when the file holds several. |
| `--operator-key <FILE>` | Operator private key. Requires `--operator-id`. |
| `--operator-id <ID>` | Stable operator identifier (email, employee id, service account). Signed. |
| `--display-name`, `--role`, `--org` | Optional operator identity fields. Signed. |
| `--proof-dir <DIR>` | Also write OpenSSL proof artifacts here (created if missing). |
| `--force` | Overwrite an existing output bundle, its `.pub` file(s) and existing `--proof-dir` artifacts. A `--force` reseal without `--operator-key` removes a stale `<OUTPUT>.operator.pub` from an earlier attested seal. |
| `--format human\|json` | Output format. Default `human`. |

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Sealed and self-verified. |
| `1` | Integrity or verification failure. |
| `2` | Usage error or bad input (for example a refused trace, an output that exists without `--force`, or an output path that is a directory). |
| `3` | I/O or environment error (for example a missing or unwritable output directory). |

## JSON output

On success, `--format json` prints one object on stdout:

```json
{
  "tool": "akmon seal",
  "akmon_version": "3.0.0",
  "passed": true,
  "session_id": "04686e85-7edd-43ae-ab4d-03f64105ae9b",
  "bundle": "/home/alice/evidence/audit.akmon",
  "signer_public_key": "dffb0278312c9cee40132dc424591f0e1cd44f5ef8f8bda02b033e99a8ec5d35",
  "signer_key_id": "327f8f3b634375deef2190a13e6224441b1b6fa2773e2a0d281970dd52e8771f",
  "capture_level": "full",
  "verify_command": "agef-verify audit.akmon --verify-key audit.akmon.pub --require-signature"
}
```

`bundle` is an absolute path. `operator_public_key` is added when attesting, `proof_dir` when `--proof-dir` is given, and `removed_stale_operator_pub` when a `--force` reseal removed a stale operator key file. Errors use the shared error object `{tool, akmon_version, passed: false, error, category}`, for example category `output_exists` (exit `2`) when an output exists without `--force`, `invalid_output` (exit `2`) when an output path is a directory, `io_error` (exit `3`) when the output directory is missing or not writable, or `input_not_found` (exit `2`) when the input is neither a file nor a session id.

## Example: seal with operator sign-off and an OpenSSL proof

```bash
akmon seal trace.json \
  --key signer.pk8 \
  --operator-key alice.pk8 --operator-id alice@example.com --role approver \
  --proof-dir proof \
  --output audit.akmon
```

## See also

- [Quick start](../getting-started/quickstart.md)
- [Hand evidence to an auditor](../guides/hand-to-auditor.md)
- [akmon otel import](./otel-import.md), [akmon bundle sign](./bundle-sign.md), [akmon bundle attest](./bundle-attest.md), [agef-verify](./agef-verify.md)
