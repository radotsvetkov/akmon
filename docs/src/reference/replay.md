# akmon replay

## Synopsis

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

## Description

`akmon replay` re-executes a recorded session using playback substitutes for providers and tools, then compares replayed events against source-session events and reports divergences.

Use replay for regression checks ("does this recorded session still drive the same agent-loop behavior?"), debugging ("walk the same evidence path again"), and determinism validation ("does the orchestrator make the same decisions given equivalent recorded outputs?").

Per extended P11, replay does not compare provider request bytes directly. Request payloads contain runtime-variable content (for example session identifiers and environment paths) that cannot be faithfully reconstructed in v2.0.0 replay. Comparison focuses on playback outputs (`response_hash`, `stream_hash`, `output_hash`) and deterministic agent-loop decisions (event sequence, call ordering, tool invocations, message structure).

## Arguments

### `<session-id>` (required)

Hyphenated UUID of the source session to replay.

## Options

### `--journal <path>` (optional)

Source journal directory. If omitted, Akmon resolves the default D-04 location (`$XDG_STATE_HOME/akmon/journal`).

### `--mode <default|strict>` (optional, default: `default`)

Replay comparison mode:

- `default`: semantic-equivalence comparison. Event kinds and ordering must match, with matching content references for comparable fields.
- `strict`: projection-hash comparison after normalization of producer-stamped fields (timestamps and attempt timing). Stricter mismatch detection than default mode for fields that are compared. In v2.0.0, both modes skip direct comparison of fields containing runtime-variable identifiers (see "Modes in detail" below for the v2.0.0 contract).

### `--persist` (optional)

Persist replay output as a new replay session in a target journal.

`--persist` requires `--persist-to <path>`.

### `--persist-to <path>` (optional, required with `--persist`)

Target journal directory for persisted replay output.

### `--format <human|json>` (optional, default: `human`)

Status output format:

- `human`: terminal-oriented replay summary with capped divergence list.
- `json`: machine-readable report/error payload.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Replay completed with no divergences (`passed: true`) |
| `1` | Replay completed with divergences (`passed: false`) |
| `2` | Usage error (invalid arguments or invalid flag combinations) |
| `3` | I/O or environment error (missing source session, malformed source, unwritable persist target, etc.) |

## Output formats

### Human (default)

```text
replay: source session 550e8400-e29b-41d4-a716-446655440000
  mode: default
  events compared: 14
  source events: 14
  replay events: 14
  primitive divergences: 0
  engine divergences: 0
  passed: yes
```

When replay is persisted:

```text
  persisted as: 7c9a3f8b-1111-2222-3333-444455556666 in /path/to/persist/journal
```

When divergences are present:

```text
  divergences:
    [1] event 5: AssistantContentMismatch
          expected: ...
          actual:   ...
    ...
    (and 23 more; use --format json for full list)
```

Human output shows the first 10 divergences and summarizes the remainder.

### JSON (`--format json`)

On success or divergence, replay writes `ReplayReportV1`:

- `akmon_version`, `agef_version`
- `source_session_id`, `source_head`
- `replay_session_id` (optional, set when `--persist` is used)
- `mode`
- `events_compared`, `source_event_count`, `replay_event_count`
- `primitive_divergence_count`, `engine_divergence_count`
- `divergences[]` (`event_seq`, `kind`, `expected`, `actual`)
- `passed`

On infrastructure failure, replay writes `ReplayInfraError`:

- `akmon_version`
- `error`
- `category`
- optional context fields: `source_session_id`, `missing_provider_id`, `missing_tool_id`, `missing_object_hash`

## Modes in detail

`default` mode prioritizes semantic equivalence and actionable divergence reporting for production replay workflows.

`strict` mode adds projection-hash checks to detect differences default mode may tolerate, while still honoring extended P11 exclusions.

For excluded fields (`ProviderCall.request_hash`, `SessionStart.config_hash`), both default and strict mode skip direct comparison in v2.0.0. Full field-level normalization inside serialized payloads is deferred to Item 5.8.

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

## What replay does NOT do

- Does not compare provider request bytes (`ProviderCall.request_hash`) in v2.0.0.
- Does not compare `SessionStart.config_hash` directly in v2.0.0.
- Does not support multi-provider source sessions in v2.0.0 (see Item 5.6).
- Does not perform live regeneration against current providers (see Item 5.4).
- Does not accept bundle files directly as replay input in v2.0.0 (import first; see Item 5.5).
- Does not re-apply recorded side effects from tools.
- Does not provide request-byte-identical replay fidelity in v2.0.0 (see Item 5.7).
- Does not implement full field-level strict normalization for serialized payloads in v2.0.0 (see Item 5.8).
- Does not allow implicit persist into source journal; `--persist` requires explicit `--persist-to`.
- Does not replace integrity verification; run `akmon verify` first when integrity assurance is required.

## Workflow notes

- Run `akmon verify <session-id>` before replay when source integrity must be established first.
- Use `akmon inspect <session-id>` (and `--resolve`) to inspect evidence shape before replay triage.
- For bundle-origin sessions, import first (`akmon bundle import ...`) and then replay by session id.

## See also

- [akmon verify](./verify.md)
- [akmon inspect](./inspect.md)
- [akmon bundle import](./bundle-import.md)
- [CLI Reference](./cli.md)
- AGEF specification: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef)
