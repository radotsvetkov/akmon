# akmon inspect

## Synopsis

Inspect one on-disk session journal and display its event contents.

```bash
akmon inspect <session-id> [OPTIONS]
```

```bash
akmon inspect <session-id> \
  [--journal <path>] \
  [--format <human|json>] \
  [--verbose] \
  [--resolve] \
  [--binary <meta|hex|base64>]
```

## Description

`akmon inspect` reads a stored session from the local Akmon journal and prints the event timeline with kind-specific fields. It is a read-only inspection command: no session data is created, modified, or deleted.

Use it when you need to review what happened in a session, debug tool/model behavior, or prepare audit evidence review. A reviewer can use inspect to see exactly what was said, what provider attempts occurred, what tools ran, and which hashes connect each step.

`akmon inspect` and `akmon verify` are complementary. `inspect` shows contents; `verify` checks integrity and tamper evidence. In v2.0.0 both are substrate-only commands and both target one session by UUID.

## Arguments

### `<session-id>` (required)

Hyphenated UUID assigned at `AgentSession` construction.

Example:

```bash
akmon inspect 550e8400-e29b-41d4-a716-446655440000
```

## Options

### `--journal <path>` (optional)

Journal directory to inspect. If omitted, Akmon resolves the default per-user journal path (`$XDG_STATE_HOME/akmon/journal`, per D-04).

### `--format <human|json>` (optional, default: `human`)

Select output format:

- `human`: terminal-friendly multi-line output.
- `json`: machine-readable `InspectReportV1`.

### `--verbose` (optional)

Expands human output from summary to full detail: full hashes, parent hashes, `emitted_at`, and full provider attempt records. Has no effect on JSON output (JSON always includes full detail).

### `--resolve` (optional)

Resolves referenced object hashes from local object storage and includes content-aware renderings (UTF-8 text or binary metadata). Without `--resolve`, inspect shows hash references only.

### `--binary <meta|hex|base64>` (optional, default: `meta`)

Display mode for non-UTF-8 resolved content:

- `meta`: `<binary, N bytes, hash: ...>`
- `hex`: first 64 bytes as lowercase hex pairs, then truncation footer if needed
- `base64`: first 128 base64 characters, then truncation footer if needed

`hex` and `base64` require `--resolve`. `meta` can be provided without `--resolve`, but has effect only when objects are resolved.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Session displayed successfully |
| `1` | Reserved (not currently emitted by `inspect`) |
| `2` | Usage error (for example, `--binary hex` without `--resolve`) |
| `3` | I/O or environment error (journal/session not found, read failure) |

## Output formats

### Human (default, summary)

```text
session: 550e8400-e29b-41d4-a716-446655440000
events: 4
journal: /home/user/.local/state/akmon/journal

[0] SessionStart hash=8b2a3f7c...
  cwd_hash: 1f3a5b2e...
  config_hash: 4e1d92a8...

[1] UserTurn hash=5c9d1e41...
  prompt_hash: d7ac9f11...

[2] ProviderCall hash=9a7f220e...
  provider: anthropic-claude
  attempts: 1 (1 Success)
  stream_hash: 0f23cd18...

[3] AssistantTurn hash=31b8f501...
  message_hash: 69ea4bd9...
```

### Human (verbose)

```text
[2] ProviderCall hash=9a7f220e7fd7f52a0b9c6ec8337f9c0da52dc1a4f8e96767bfac44e5f3c4f2d0
  parent: 5c9d1e41c9eb7b468b3f31c30d0495a6708ec61862db2f3ea1df1c53de2b9581
  emitted_at: 2026-01-15T14:32:10.123Z
  provider: anthropic-claude
  attempts:
    [1] status=Success started=14:32:10.123 ended=14:32:12.234
      request_hash: 0a4d4c95b4...
      response_hash: a4fbc71e2c...
      stream_hash: 0f23cd18b1...
```

### Human (resolve, text)

```text
[1] UserTurn hash=5c9d1e41...
  prompt_hash: d7ac9f11...
  prompt:
    | how do I configure the policy engine to allow shell
    | commands matching a specific prefix without prompting
    | each time?
```

### Human (resolve, binary)

```text
[4] ToolCall hash=cc77e1ab...
  tool: read_file
  input_hash: a1b2c3d4...
  input: <binary, 1024 bytes, hash: a1b2c3d4...>
    | a1 b2 c3 d4 e5 f6 ... (truncated, 960 more bytes)
```

### JSON (`--format json`)

```json
{
  "akmon_version": "1.8.2",
  "agef_version": "0.1.1",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "journal_path": "/home/user/.local/state/akmon/journal",
  "events": [
    {
      "sequence": 1,
      "event_hash": "5c9d1e41c9eb7b468b3f31c30d0495a6708ec61862db2f3ea1df1c53de2b9581",
      "parent_hashes": [
        "8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757"
      ],
      "emitted_at": "2026-01-15T14:32:09.942Z",
      "kind": {
        "type": "user_turn",
        "prompt_hash": "d7ac9f11f8069ce39f5df1863fcef84f8f46406fdb6f866f9317dbf2ca6fcb53",
        "prompt_text": "how do I configure the policy engine?",
        "prompt_size": 43
      }
    }
  ]
}
```

#### `InspectReportV1`

- `akmon_version`: Akmon CLI version that produced this report.
- `agef_version`: AGEF specification version implemented by the substrate.
- `session_id`: session UUID (hyphenated lowercase).
- `journal_path`: resolved absolute journal directory used.
- `events`: array of `InspectEvent` in sequence order.

#### `InspectEvent`

- `sequence` (`u64`): event sequence number (0-indexed).
- `event_hash` (`string`): lowercase hex-encoded event hash.
- `parent_hashes` (`string[]`): lowercase hex-encoded parent hashes.
- `emitted_at` (`string`): ISO 8601 UTC timestamp.
- `kind` (`InspectEventKind`): tagged event payload.

#### `InspectEventKind`

`kind` is a tagged enum with `type` discriminator (`snake_case`):

- `session_start`: `cwd_hash`, `config_hash`, and with `--resolve` optional `cwd_text`/`cwd_size`, `config_text`/`config_size`
- `user_turn`: `prompt_hash`, and with `--resolve` optional `prompt_text`/`prompt_size`
- `provider_call`: `provider_id`, `attempts`, `stream_hash`, and with `--resolve` optional `stream_text`/`stream_size`
- `tool_call`: `tool_id`, `input_hash`, `output_hash`, `side_effects_hash`, and with `--resolve` optional `input_text`/`input_size`, `output_text`/`output_size`, `side_effects_text`/`side_effects_size`
- `retrieval_call`: `index_id`, `query_hash`, `results_hash`, and with `--resolve` optional `query_text`/`query_size`, `results_text`/`results_size`
- `permission_gate`: `policy_id`, `decision`, `context_hash`, and with `--resolve` optional `context_text`/`context_size`
- `assistant_turn`: `message_hash`, `tool_calls_hash`, and with `--resolve` optional `message_text`/`message_size`, `tool_calls_text`/`tool_calls_size`
- `session_end`: `summary_hash`, and with `--resolve` optional `summary_text`/`summary_size`

#### `InspectAttempt`

- `attempt_number` (`u32`): 1-indexed attempt number.
- `status` (`string`): attempt status name.
- `started_at` (`string`): ISO 8601 UTC timestamp.
- `ended_at` (`string`): ISO 8601 UTC timestamp.
- `request_hash` (`string`): lowercase hex-encoded request hash.
- `response_hash` (`string | null`): response hash when present.
- `stream_hash` (`string | null`): stream transcript hash when present.
- `error_message` (`string | null`): provider error message when present.
- `request_text`/`request_size`, `response_text`/`response_size`, `stream_text`/`stream_size`: present only with `--resolve` and only when content is available.

#### `InspectError`

Infrastructure failures use this JSON shape:

```json
{
  "akmon_version": "1.8.2",
  "category": "session_not_found",
  "error": "cannot open journal ...: session not found: 550e8400-e29b-41d4-a716-446655440000"
}
```

- `akmon_version`: Akmon CLI version that produced the error.
- `category`: one of:
  - `journal_not_found`
  - `session_not_found`
  - `inspect_infrastructure_error`
- `error`: human-readable diagnostic message.

## Examples

### 1) Inspect a session (default summary)

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000
```

### 2) Verbose inspection

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000 --verbose
```

### 3) Show resolved content (text and binary metadata)

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000 --resolve
```

### 4) Show binary payloads as hex preview

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000 --resolve --binary hex
```

### 5) JSON output for CI

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000 --format json | jq '.events | length'
```

### 6) JSON with resolved user text

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000 --format json --resolve \
  | jq '.events[] | select(.kind.type == "user_turn") | .kind.prompt_text'
```

### 7) Custom journal path

```bash
$ akmon inspect 550e8400-e29b-41d4-a716-446655440000 --journal /tmp/my-journal
```

## What inspect shows

`inspect` can display these event kinds:

- `SessionStart`
- `UserTurn`
- `ProviderCall` (including attempt details)
- `ToolCall`
- `PermissionGate`
- `AssistantTurn`
- `SessionEnd`
- `RetrievalCall` (if present in journal; reserved for future Akmon emission)

`RetrievalCall` support is included for forward compatibility with AGEF and v2.0 planning; Akmon v2.0.0 does not emit it in normal runs.

## What inspect does not do

- It does not verify integrity or tamper evidence (`akmon verify` does that).
- It does not modify session state (read-only journal access).
- It does not fetch content from external systems (resolution reads only local journal object bytes).
- It does not decode domain-specific binary encodings beyond UTF-8 detection and preview rendering.

## Programmatic / CI usage

- Use `--format json` and `jq` (or your parser) to query events by kind and field.
- Use exit codes (`0`/`2`/`3`) to handle success, usage issues, and missing journal/session cases.
- `--resolve` increases read cost with total resolved object size; skip when hashes are sufficient.
- Output is deterministic for the same session contents (covered by integration test `t_inspect_output_stability`).

## See also

- `akmon verify`: [./verify.md](./verify.md)
- AGEF specification: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef)
- `akmon export` / `akmon import` (Item 4.3, coming)
