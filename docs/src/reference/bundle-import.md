# akmon bundle import

Documented for Akmon `2.1.0`.

## Who this is for

Teams validating and ingesting portable `.akmon` bundles into local journals.

## What you will have at the end

- A verified bundle (`--verify-only`) or imported session.
- Clear handling for collisions and archive validation failures.

## Prerequisites

- Path to a `.akmon` bundle file.
- Write access to target journal if ingesting.

## Steps

```bash
akmon bundle import <bundle-path> [OPTIONS]
```

```bash
akmon bundle import <bundle-path> \
  [--journal <path>] \
  [--format <human|json>] \
  [--verify-only] \
  [--allow-extra-files] \
  [--rename-to <NEW_UUID>]
```

1. Validate bundle only (no local writes):

```bash
akmon bundle import /path/to/audit.akmon --verify-only
```

2. Import into journal:

```bash
akmon bundle import /path/to/audit.akmon
```

3. Resolve session-id collisions with rename:

```bash
akmon bundle import /path/to/audit.akmon --rename-to <new-uuid>
```

4. Use JSON output in CI:

```bash
akmon bundle import /path/to/audit.akmon --verify-only --format json
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Bundle imported successfully (or verified successfully with `--verify-only`) |
| `1` | Bundle validation failed (AGEF integrity/structure violation) |
| `2` | Usage/recoverable import error (for example, session collision without suitable `--rename-to`) |
| `3` | I/O or environment error (bundle not found, unwritable journal, local store corruption) |

## Verification

```bash
akmon bundle import /path/to/audit.akmon --verify-only --format json | jq '.passed'
```

Expected result: `true` for valid bundle, otherwise `false` with violations and exit `1`.

## Validation checks (AGEF alignment)

Import validation aligns with AGEF structural/integrity requirements, including:

- Manifest parse/schema-required fields
- `events.bin` frame decoding (length-prefixed canonical CBOR events)
- Event hash-chain integrity
- Object closure (all referenced hashes present)
- Object byte re-hash (bytes match hash key)
- Head consistency (`manifest.session.head` matches terminal event hash)
- Session boundary invariants (`SessionStart` first, `SessionEnd` terminal)
- Sequence continuity (`0..n-1`)
- Strict unknown-content handling by default (unknown event tags/statuses/extra archive files rejected unless flags permit)

## Troubleshooting

- If import exits `2` for collision, rerun with `--rename-to <NEW_UUID>`.
- If verify-only exits `1`, inspect JSON `violations` categories.
- If import exits `3`, check bundle path, journal permissions, and disk availability.

## See also

- [agef-verify](./agef-verify.md) — standalone bundle verifier (no Akmon CLI)
- `akmon bundle export`: [./bundle-export.md](./bundle-export.md)
- `akmon verify`: [./verify.md](./verify.md)
- `akmon inspect`: [./inspect.md](./inspect.md)
- [AGEF specification](https://github.com/radotsvetkov/agef)
