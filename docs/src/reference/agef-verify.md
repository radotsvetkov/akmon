# agef-verify

Documented for Akmon `3.0.0`.

`agef-verify` is a small, standalone verifier for `.akmon` bundles. It is what you give an auditor. It does not include the agent, the journal, or any network code, and it runs fully offline.

## Install

```bash
brew tap radotsvetkov/akmon
brew install agef-verify
```

or download the `agef-verify-*` binary from a GitHub release and check it against `SHA256SUMS` ([Install](../getting-started/installation.md)).

## Usage

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature
agef-verify audit.akmon --verify-key signer.pub --require-signature --format markdown > report.md
agef-verify audit.akmon --format json
```

## Options

The same verification flags as [`akmon bundle verify`](./bundle-verify.md):

| Option | Meaning |
| --- | --- |
| `--verify-key <HEX_FILE>` | Trusted signer public key (64 hex chars in a file). Repeatable. |
| `--require-signature` | Fail unless a signature verifies against a `--verify-key`. |
| `--operator-key <HEX_FILE>` | Trusted operator public key. Repeatable. |
| `--require-operator` | Fail unless an operator attestation verifies. |
| `--require-operator-key <HEX_FILE>` | Fail unless this specific key attested. Repeatable. |
| `--require-capture full` | Fail unless the session captured full message content. |
| `--allow-extra-files` | Tolerate unknown files in the archive. |
| `--max-decoded-bytes <BYTES>` | Decompression limit. Default `1073741824` (1 GiB). |
| `--format human\|json\|markdown` | `human` on stderr (default), `json` on stdout, or a Markdown auditor report on stdout. |

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Passed. Without `--require-signature` this means **integrity only**; check `signature_verified` in JSON. |
| `1` | Failed, including a malformed, corrupt, truncated, or oversized archive. |
| `2` | Usage error. |
| `3` | I/O or environment error: bundle or key file missing or unreadable, invalid key file. |

## Output

The human headline, the JSON report (`BundleVerifyReportV1`), and the Markdown report are the same as `akmon bundle verify`; see [Reading the result](./bundle-verify.md#reading-the-result). `akmon_version` in the JSON is the `agef-verify` version.

Errors use the shared error object:

```json
{
  "tool": "agef-verify",
  "akmon_version": "3.0.0",
  "passed": false,
  "error": "cannot resolve bundle path audit.akmon: No such file or directory (os error 2)",
  "category": "io_error"
}
```

In v3.0.0 the field `tool_version` was renamed to `akmon_version`, and every error now carries `tool` and `passed: false`. Update any automation that parsed the old name.

## See also

- [Hand evidence to an auditor](../guides/hand-to-auditor.md)
- [Verify on an air-gapped machine](../use-cases/air-gapped-audit.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
