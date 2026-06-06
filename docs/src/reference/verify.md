# akmon verify

Documented for Akmon `2.2.0`.

## Who this is for

Reviewers, CI engineers, and operators validating recorded session integrity before trusting artifacts.

## What you will have at the end

- A pass/fail integrity decision for one session UUID.
- Optional JSON output suitable for CI gates.

## Prerequisites

- A session UUID from a completed Akmon run.
- Access to the journal directory (default or custom).

## Steps

1. Run verification on a session UUID.

```bash
akmon verify <session-id>
```

2. Use JSON for automation.

```bash
akmon verify <session-id> --format json
```

3. Use optional flags as needed:
   - `--journal <PATH>`
   - `--format <human|json>` (default `human`)
   - `--verbose`

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Verification succeeded (no violations) |
| `1` | Verification completed and found violations |
| `2` | Usage error (argument parsing/CLI contract) |
| `3` | I/O or environment error (journal/session/infrastructure failure) |

## Verification

```bash
akmon verify <session-id> --format json | jq '.passed'
```

Expected result: `true` for an intact session; non-zero exit with violations/errors otherwise.

## What verify checks (AGEF Section 13 alignment)

- **Parent chain integrity**: each non-start event points to the expected prior event.
- **Sequence integrity**: event sequences are contiguous (`0..n-1`).
- **Event hash recompute**: canonical CBOR event bytes hash to stored event hashes.
- **Object presence**: each referenced object hash resolves in object storage.
- **Object byte re-hash**: resolved object bytes hash back to referenced object hashes.
- **Head consistency**: stored session head equals the terminal event hash.
- **SessionEnd invariants**: exactly one `SessionEnd`, and it is terminal.

## Troubleshooting

- `exit 2`: invalid CLI usage; re-check `akmon verify --help`.
- `exit 3`: session/journal access error; verify UUID and `--journal` path.
- `exit 1`: integrity violation found; inspect JSON `violations` for exact category.

## See also

- [AGEF specification](https://github.com/radotsvetkov/agef)
- [akmon inspect](./inspect.md)
- [akmon replay](./replay.md)
