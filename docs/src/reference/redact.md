# akmon redact

Documented for Akmon `2.2.0`.

## Who this is for

Teams generating sanitized derivative bundles for external review without exposing sensitive object content.

## What you will have at the end

- A derivative `.akmon` bundle with selected objects replaced by redaction sentinels.
- A reproducible command trail with explicit rationale (`--reason`).

## Prerequisites

- Source session UUID.
- Object hashes to redact (typically found via `akmon inspect --resolve`).
- Writable destination path for `--output`.

## Steps

```bash
akmon redact <session-id> [OPTIONS]
```

```bash
akmon redact <session-id> \
  --output <path> \
  --object <hash> [--object <hash> ...] \
  --reason <text> \
  [--journal <path>] \
  [--format <human|json>]
```

1. Create sanitized derivative bundle:

```bash
akmon redact <session-id> \
  --output sanitized.akmon \
  --object <object-hash> \
  --reason "PII removal"
```

2. For multiple objects, repeat `--object`.

3. Verify derivative bundle before sharing:

```bash
akmon bundle import sanitized.akmon --verify-only
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Derivative bundle written successfully |
| `1` | Reserved (not currently emitted by `redact`) |
| `2` | Usage error (output exists, invalid hash format, object not in session, missing required flag) |
| `3` | I/O or environment error (journal/session not found, write failure, unreadable referenced object) |

## Verification

```bash
akmon redact <session-id> --output sanitized.akmon --object <object-hash> --reason "compliance" --format json | jq '.objects_redacted_count'
```

Expected result: positive redacted-object count and exit `0`.

## Sentinel format

Redacted objects are replaced by canonical-CBOR sentinel objects with this payload:

```json
{
  "akmon_redacted": true,
  "original_hash": "<hex of original>",
  "original_size": 1024,
  "reason": "<text from --reason>",
  "redacted_at": "<RFC3339 timestamp>"
}
```

## Troubleshooting

- If output path exists, choose a new `--output` target.
- If `--object` is rejected, confirm lowercase hex hash and that it is referenced in source session.
- Redaction does not verify source integrity automatically; run `akmon verify <session-id>` first when required.

## See also

- [akmon verify](./verify.md)
- [akmon inspect](./inspect.md)
- [akmon bundle export](./bundle-export.md)
- [akmon bundle import](./bundle-import.md)
- [AGEF specification](https://github.com/radotsvetkov/agef)
