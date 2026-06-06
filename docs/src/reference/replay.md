# akmon replay

Documented for Akmon `2.2.0`.

## Who this is for

Engineers replaying recorded sessions to detect divergences and regression behavior.

## What you will have at the end

- A replay pass/fail report (`default` or `strict` mode).
- Optional persisted replay session in a target journal.

## Prerequisites

- Source session UUID.
- Journal access to source session.

## Steps

```bash
akmon replay <session-id> [OPTIONS]
```

```bash
akmon replay <session-id> \
  [--journal <path>] \
  [--mode <default|strict>] \
  [--persist --persist-to <path>] \
  [--format <human|json>]
```

1. Run default replay:

```bash
akmon replay <session-id>
```

2. Use strict mode if you want tighter mismatch handling:

```bash
akmon replay <session-id> --mode strict
```

3. Use JSON in CI:

```bash
akmon replay <session-id> --format json
```

4. Persist replay output only with explicit target:

```bash
akmon replay <session-id> --persist --persist-to /path/to/replay-journal
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Replay completed with no divergences (`passed: true`) |
| `1` | Replay completed with divergences (`passed: false`) |
| `2` | Usage error (invalid arguments or invalid flag combinations) |
| `3` | I/O or environment error (missing source session, malformed source, unwritable persist target, etc.) |

## Verification

```bash
akmon replay <session-id> --format json | jq '.passed'
```

Expected result: `true` for equivalent replay; `false` if divergences are detected.

## Examples

### 1) Replay a session with default mode

```bash
$ akmon replay 550e8400-e29b-41d4-a716-446655440000
```

### 2) Replay in strict mode

```bash
$ akmon replay 550e8400-e29b-41d4-a716-446655440000 --mode strict
```

### 3) JSON output for CI

```bash
$ akmon replay 550e8400-e29b-41d4-a716-446655440000 --format json | jq '.passed'
```

### 4) Persist replay output in a target journal

```bash
$ akmon replay 550e8400-e29b-41d4-a716-446655440000 \
  --persist \
  --persist-to ./replay-journal
```

### 5) Replay from a non-default journal

```bash
$ akmon replay 550e8400-e29b-41d4-a716-446655440000 --journal /custom/journal
```

## Troubleshooting

- If `--persist` fails, ensure `--persist-to` is provided and writable.
- If replay cannot load source data, validate `--journal` path and source session UUID.
- If replay fails with divergences, inspect JSON `divergences` for event-level mismatch details.
- For integrity-first workflow, run `akmon verify <session-id>` before replay.

## See also

- [akmon verify](./verify.md)
- [akmon inspect](./inspect.md)
- [akmon bundle import](./bundle-import.md)
- [CLI Reference](./cli.md)
- [AGEF specification](https://github.com/radotsvetkov/agef)
