# akmon bundle verify

Documented for Akmon `3.0.0`.

Verify a `.akmon` bundle without touching the local journal. It runs the same checks as the standalone [`agef-verify`](./agef-verify.md) and takes the same flags.

## Usage

```bash
akmon bundle verify audit.akmon
akmon bundle verify audit.akmon --verify-key signer.pub --require-signature
akmon bundle verify audit.akmon --verify-key signer.pub --format markdown > report.md
akmon bundle verify audit.akmon --format json
```

## What it checks

- Manifest schema, event framing, and that every object re-hashes to its name.
- The hash chain from the first event to the session head, and that the manifest head matches.
- Object closure: every referenced object is present, and no unreferenced object is (`unreferenced_object`).
- Provider-call invariants inside events (`event_invariant`).
- Archive strictness: duplicate entries, badly named objects, and (by default) any file outside `manifest.json`, `events.bin`, and `objects/<hex>` fail verification.
- With `--verify-key`: each signature in `manifest.signatures[]` against the keys you trust.
- With `--operator-key`: each operator attestation against the operator keys you trust.

**Without `--require-signature`, a pass means integrity only.** Anyone can build an internally consistent bundle. Authorship needs a key you trust.

## Options

| Option | Meaning |
| --- | --- |
| `--verify-key <HEX_FILE>` | Trusted signer public key (64 hex chars in a file). Repeatable. |
| `--require-signature` | Fail unless at least one signature verifies against a `--verify-key`. |
| `--operator-key <HEX_FILE>` | Trusted operator public key. Repeatable. |
| `--require-operator` | Fail unless at least one operator attestation verifies against an `--operator-key`. |
| `--require-operator-key <HEX_FILE>` | Fail unless this specific key attested. Repeatable; also trusts the key. |
| `--require-capture full` | Fail unless the session captured full message content. Rejects `partial` and `structural`. |
| `--allow-extra-files` | Tolerate unknown files in the archive. Default is strict reject. |
| `--max-decoded-bytes <BYTES>` | Decompression limit. Default `1073741824` (1 GiB). |
| `--format human\|json\|markdown` | `human` on stderr (default), `json` on stdout, or a Markdown auditor report on stdout. |

## Reading the result

The human headline states exactly what was established:

| Headline | Meaning |
| --- | --- |
| `verified bundle: integrity OK; signature verified by key_id=<id>` | Contents match the head, and a trusted key signed it. |
| `verified bundle: integrity OK; signature present but NOT verified (…integrity only)` | Signed, but not by any key you supplied. |
| `verified bundle: integrity OK; UNSIGNED (integrity only)` | No signature at all. |
| `bundle verification FAILED` | Something does not match. The violations list says what. |

The body shows the head, the session window (first and last event timestamps, from the producer's clock; for an OTEL import this is the import time, see [otel import](./otel-import.md#timestamps)), capture level, signatures, operators, and the **unsigned metadata** (`created_at`, `ended_at`, producer). That metadata is not covered by any signature. If it disagrees with the event timestamps you get a warning, not a failure.

`--format markdown` writes the same result as a report with a verdict, a checks table, and "What this proves" and "What this does not prove" sections. Untrusted values from the bundle are escaped.

## JSON output

`--format json` prints `BundleVerifyReportV1`. Main fields:

| Field | Meaning |
| --- | --- |
| `passed` | The overall verdict, including any `--require-*` flags. |
| `integrity_ok` | The hash chain, objects, and head are consistent. |
| `signature_verified` | At least one signature verified against a trusted key. Read it together with `integrity_ok` and `passed`. |
| `operator_key_verified` | At least one operator attestation verified against a trusted key. |
| `head`, `hash_algorithm` | The session head and its algorithm. |
| `session_window` | `first_event_at` and `last_event_at`. Omitted when there are no events. |
| `capture` | `level` (`full`, `partial`, `structural`) and `source_semconv` for imports. |
| `signatures[]`, `operators[]` | Per-entry outcomes. Operator identity strings are self-asserted. |
| `unsigned_metadata` | `created_at`, `ended_at`, `producer_name`, `producer_version`. Not signed. |
| `warnings[]` | For example `manifest_created_at_mismatch`. Never fail verification. |
| `violations[]` | Why verification failed, each with a `category`. |

Errors that stop verification before a report exists (unreadable file, bad key file) use the shared error object `{tool, akmon_version, passed: false, error, category}`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Passed: integrity plus any `--require-*` requirements. |
| `1` | Failed, including a malformed, corrupt, truncated, or oversized archive. |
| `2` | Usage error. |
| `3` | I/O or environment error, for example a missing bundle or an invalid key file. |

## Operator identity

"Verified" attaches to the key, never to the name. The `operator_id`, `role`, and `org` strings are whatever the signer typed. Binding a key to a real person is something you establish out of band. See [Operator sign-off](../use-cases/operator-sign-off.md).

## See also

- [agef-verify](./agef-verify.md)
- [What verification proves](../concepts/verifying-evidence.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
- [akmon bundle import](./bundle-import.md) (`--verify-only` runs the same checks)
