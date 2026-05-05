# akmon diff

## Synopsis

```bash
akmon diff <session-a> <session-b> [OPTIONS]
```

```bash
akmon diff 550e8400-e29b-41d4-a716-446655440000 6ba7b810-9dad-11d1-80b4-00c04fd430c8 \
  [--journal <path>] \
  [--resolve] \
  [--format <human|json>]
```

## Description

`akmon diff` compares two recorded sessions in the same journal scope and reports structural and field-level divergences. Both UUIDs must exist under the selected journal directory (default: `$XDG_STATE_HOME/akmon/journal` unless `--journal` is set).

Use diff for evidence-side regression checks (two runs of the same workflow), replay validation (source session vs replayed session in one store), and audit explanations (“what changed between these two session heads?”).

Comparison is lockstep by event sequence. When event kinds or counts diverge, diff reports a structural break and stops further alignment for that pair.

## Arguments

### `<session-a>`, `<session-b>` (required)

Hyphenated UUIDs of the two sessions to compare.

## Options

### `--journal <path>` (optional)

Journal directory containing `journal.redb` for **both** sessions. If omitted, Akmon uses the default journal location.

### `--resolve` (optional)

Dereference content hashes for comparable fields and attach byte-level summaries (`resolved` or `resolved_skip_reason` on each divergence in JSON). Without this flag, comparison uses hash and structural summaries only.

### `--format <human|json>` (optional, default: `human`)

- `human`: terminal-oriented summary; divergences list is capped at 10 entries (same cap as `akmon replay` human output), with a footer pointing to JSON for the full list.
- `json`: `DiffReportV1` as pretty-printed JSON on standard output.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Compared successfully; sessions are equivalent (`matches: true`) |
| `1` | Compared successfully; divergences or structural break (`matches: false`) |
| `2` | Usage error (for example malformed session UUID caught as a load error in edge paths; most parse failures exit via Clap with code 2) |
| `3` | Infrastructure error (journal resolution failure, missing session, load/precondition failure, store access failure, print failure) |

## Output formats

### Human (default)

First lines mirror the replay report shape: command line, indented stats (`mode`, `events compared`, per-session event counts, `divergence count`, `matches: yes|no`). When comparison fails, optional `structural break:` and `divergences:` sections follow, with `expected:` / `actual:` lines per divergence. With `--resolve`, divergences may include `resolved:` byte summaries or `resolve skipped:` reasons.

### JSON

Pretty-printed `DiffReportV1` (schema owned by the `akmon-diff` crate). Suitable for CI ingestion and golden tests.

## See also

- [akmon replay](./replay.md) — replay comparison and exit-code discipline aligned with diff
- [akmon inspect](./inspect.md) — single-event inspection; `--resolve` preview rules are aligned with diff resolve mode

---

**Phase 6 (Item 6.3).** v2.0.0 ships CLI diff with human and JSON reporting, integration tests, and a single shared `journal.redb` open when loading both sessions so redb per-process locking is respected.
