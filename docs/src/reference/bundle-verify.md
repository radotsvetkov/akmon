# akmon bundle verify

Documented for Akmon `2.1.0`.

## Who this is for

Reviewers and CI jobs that need to validate a portable `.akmon` bundle **without** writing to a
local journal. This is the preferred Akmon entrypoint for bundle-only verification (Item 4.3).

## What you will have at the end

- Confirmation that the bundle's objects, event chain, and manifest head are internally
  consistent, or a structured violation list.

## Prerequisites

- Path to a `.akmon` bundle file.

## Steps

```bash
akmon bundle verify /path/to/audit.akmon
```

```bash
akmon bundle verify /path/to/audit.akmon --format json
```

Optional flags:

- `--allow-extra-files` — tolerate unknown files inside the archive (default is strict reject).
- `--format human|json` — default `human`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Bundle passed all integrity checks |
| `1` | Verification failed |
| `3` | I/O or environment error |

## Verification

```bash
akmon bundle verify /path/to/audit.akmon --format json | jq '.passed'
```

Expected result: `true` for a valid exported bundle.

## Equivalents

| Command | Notes |
| --- | --- |
| `akmon bundle verify` | Preferred; no journal access |
| `akmon bundle import --verify-only` | Legacy alias; same checks and JSON schema |
| `agef-verify` | Standalone binary; no Akmon CLI |

## See also

- [akmon bundle import](./bundle-import.md)
- [agef-verify](./agef-verify.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
- [akmon verify](./verify.md)
