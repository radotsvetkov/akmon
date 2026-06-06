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

## Operator identity (`--operator-key`)

[`akmon bundle attest`](./bundle-attest.md) records signed operator attestations on a bundle. To
check them at verify time:

- `--operator-key <HEX_FILE>` — a trusted operator Ed25519 public key (64 hex chars). Repeatable.
  Each `manifest.operator_attestations[]` entry is verified against the supplied keys; a matching,
  cryptographically valid attestation reports outcome `verified`. An attested bundle verified
  **without** a trusted key reports `unverified_no_key` — not a failure on its own.
- `--require-operator` — fail (exit 1) unless **at least one** operator attestation verifies against
  an `--operator-key`.
- `--require-operator-key <HEX_FILE>` — fail unless **that specific** key has a verified attestation.
  Repeatable; each listed key is also trusted for verification.

"Verified" attaches to the **key**, not the name. The JSON `operators[]` entries carry the
self-asserted `operator_id`/`role`/`org` strings verbatim, but the only trust signal is the distinct
boolean `operator_key_verified` (outcome `verified`) against a key **you** supplied. A self-asserted
identity never reads as key-verified without a trusted key; trust in the name is out-of-band.

```bash
akmon bundle verify /path/to/audit.akmon --operator-key operator.pub.hex --require-operator --format json
```

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
- [akmon bundle attest](./bundle-attest.md)
- [agef-verify](./agef-verify.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
- [akmon verify](./verify.md)
