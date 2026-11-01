# akmon diff

Documented for Akmon `3.0.0`.

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

Control characters in values and previews are shown escaped (`\u{..}`), and newlines in single-line fields as `\n`, so a crafted session cannot rewrite your terminal.

### JSON

Pretty-printed `DiffReportV1` (schema owned by the `akmon-diff` crate). Suitable for CI ingestion and golden tests.

When diff cannot run, `--format json` prints a `DiffErrorV1` object on stdout and nothing on stderr:

```json
{
  "akmon_version": "3.0.0",
  "category": "session_not_found",
  "error": "source session not found: 550e8400-e29b-41d4-a716-446655440000"
}
```

Categories: `session_not_found` (also when the journal is missing), `session_load_failed`, `source_precondition_violated`, `store_access_failed`, `diff_infrastructure_error`.

A malformed session id (not a UUID) is a usage error caught by argument parsing before diff runs: it prints a plain error on stderr, no JSON, and exits `2`.

## Provider-call divergences

Besides event kinds and fields, diff compares every provider-call attempt:

| Divergence kind | Field | Values |
| --- | --- | --- |
| `provider_call_request_difference` | `attempts[i].request_hash` | Request hashes |
| `provider_call_attempt_count_difference` | `attempts.len` | Attempt counts |
| `provider_call_attempt_status_difference` | `attempts[i].status` | `success`, `rate_limited`, `network_error`, `server_error`, `client_error`, `cancelled`, `other:<s>` |
| `provider_call_attempt_error_difference` | `attempts[i].error_message` | Error text, or `none` |

Two sessions that differ only in a retried request or an attempt outcome now diverge (exit `1`); before v3.0.0 they could report `matches: yes`.

## See also

- [akmon replay](./replay.md): replay comparison and exit-code discipline aligned with diff
- [akmon inspect](./inspect.md): single-event inspection; `--resolve` preview rules are aligned with diff resolve mode
