# agef-verify

Documented for Akmon `2.1.0`.

## Who this is for

Auditors, compliance reviewers, and CI pipelines that must verify an AGEF `.akmon` bundle
**without installing or running the Akmon agent CLI**. `agef-verify` is a minimal binary that
depends only on `akmon-bundle` (manifest, framing, objects, and store-independent integrity
checks).

## What you will have at the end

- Confirmation that a portable bundle's objects, event chain, and manifest head are internally
  consistent, or a structured list of violations.

## Prerequisites

- A `.akmon` bundle file on disk.

## Usage

```bash
agef-verify /path/to/audit.akmon
```

```bash
agef-verify /path/to/audit.akmon --format json
```

Optional flags:

- `--allow-extra-files` — tolerate unknown files inside the archive (same semantics as
  `akmon bundle import`).
- `--format human|json` — default `human`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Bundle passed all integrity checks |
| `1` | Bundle read succeeded but verification failed (or non-I/O parse/integrity error) |
| `3` | I/O or environment error (path not found, not a file, cannot render JSON) |

## JSON output

`--format json` emits **BundleVerifyReportV1**, the same shape as
`akmon bundle import --verify-only --format json`, so automation can share `jq` filters. The
`akmon_version` field carries the `agef-verify` crate version.

```bash
agef-verify /path/to/audit.akmon --format json | jq '.passed'
```

Infrastructure errors (cannot open or parse the archive) emit **VerifyInfraErrorV1** with
`tool: "agef-verify"`.

## Relation to Akmon

| Tool | Scope |
| --- | --- |
| `akmon verify <session-id>` | On-disk journal / redb store |
| `akmon bundle verify` | Same bundle checks as `agef-verify`, embedded in Akmon |
| `akmon bundle import --verify-only` | Legacy alias of `bundle verify` |
| `agef-verify` | Bundle file only; no journal, no agent |

## See also

- [akmon bundle import](./bundle-import.md)
- [akmon verify](./verify.md)
