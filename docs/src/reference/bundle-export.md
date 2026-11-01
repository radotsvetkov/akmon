# akmon bundle export

Documented for Akmon `3.0.0`.

## Who this is for

Teams exporting a portable, verifiable session artifact for handoff, audit, or archive.

## What you will have at the end

- A `.akmon` bundle generated from one journal session.
- Optional JSON metadata for automation.

## Prerequisites

- Source session UUID.
- Access to source journal and write permission for output path.

## Steps

```bash
akmon bundle export <session-id> [OPTIONS]
```

```bash
akmon bundle export <session-id> \
  [--output <path>] \
  [--journal <path>] \
  [--format <human|json>]
```

1. Export with defaults:

```bash
akmon bundle export <session-id>
```

2. Export to explicit path:

```bash
akmon bundle export <session-id> --output /path/to/audit.akmon
```

3. Use JSON in CI:

```bash
akmon bundle export <session-id> --format json
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Bundle written successfully |
| `1` | Reserved (not currently emitted) |
| `2` | Usage error (for example, output path already exists) |
| `3` | I/O or environment error (journal/session not found, missing object in store, write failure) |

## Verification

```bash
akmon bundle export <session-id> --format json | jq '.output_path'
```

Expected result: output path is returned and command exits `0`.

## Bundle format

An `.akmon` bundle is a `tar.zst` archive containing:

- `manifest.json`
- `events.bin`
- `objects/<hex>`

## Troubleshooting

- If export fails with output-exists error, choose a new `--output` path.
- If export fails with session/journal errors, verify UUID and `--journal` location.
- Export does not sign. Use [`akmon bundle sign`](./bundle-sign.md), or [`akmon seal`](./seal.md) to export, sign, and verify in one step.
- If the journal is in use by another process, JSON errors report category `journal_locked` (exit `3`).

## See also

- [akmon seal](./seal.md)
- `akmon bundle import`: [./bundle-import.md](./bundle-import.md)
- `akmon verify`: [./verify.md](./verify.md)
- `akmon inspect`: [./inspect.md](./inspect.md)
- [AGEF specification](https://github.com/radotsvetkov/agef)
