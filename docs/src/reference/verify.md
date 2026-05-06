# akmon verify

## Synopsis

Verify one on-disk session journal for integrity and tamper evidence.

```bash
akmon verify <session-id> [--journal <path>] [--format <human|json>] [--verbose]
```

## Description

`akmon verify` checks that a stored session is internally consistent and that referenced content-addressed objects are intact. It is designed for developers, reviewers, and CI systems that need defensible evidence that a captured session has not been tampered with.

The command verifies one session by UUID against an Akmon journal directory. By default it uses the per-user D-04 journal location, but you can point it at an alternate path with `--journal`.

Verification is structural and cryptographic, not semantic. A successful result means the recorded artifact is self-consistent and hash-valid; it does not mean the model behavior was correct for business intent.

## Arguments and flags

### `<session-id>` (required)

Hyphenated UUID assigned at `AgentSession` construction.

Example:

```bash
akmon verify 550e8400-e29b-41d4-a716-446655440000
```

### `--journal <path>` (optional)

Journal directory to verify against. If omitted, Akmon resolves the default per-user journal path (D-04).

Example:

```bash
akmon verify 550e8400-e29b-41d4-a716-446655440000 --journal /tmp/my-journal
```

### `--format <human|json>` (optional, default: `human`)

Select output format:

- `human`: concise terminal-oriented summary
- `json`: machine-readable report for CI/programmatic use

Example:

```bash
akmon verify 550e8400-e29b-41d4-a716-446655440000 --format json
```

### `--verbose` (optional)

Adds detailed human output. Has no effect on JSON output (JSON already includes full violation detail).

Example:

```bash
akmon verify 550e8400-e29b-41d4-a716-446655440000 --verbose
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Verification succeeded (no violations) |
| `1` | Verification completed and found violations |
| `2` | Usage error (argument parsing/CLI contract) |
| `3` | I/O or environment error (journal/session/infrastructure failure) |

## Output formats

### Human (default)

#### Clean session

```text
verified: session 550e8400-e29b-41d4-a716-446655440000
  events checked: 14
  objects checked: 31
  SessionEnd: present and terminal
```

#### Broken session

```text
verification failed: session 550e8400-e29b-41d4-a716-446655440000
  events checked: 14
  objects checked: 31

  violations:
    - missing objects: 0
    - object hash mismatches: 1
    - event hash mismatches: 0
    - parent chain breaks: 0
    - sequence violations: 0
    - head mismatch: false
    - SessionEnd: present and terminal
```

#### `--verbose` additions

- Clean sessions include a `checks performed` list from the verification report.
- Broken sessions include per-violation detail (full hashes and contextual details where available, such as referencing event hash for missing objects).

### JSON (`--format json`)

JSON is always emitted to stdout (including failure cases), so pipelines can parse deterministically.

#### `VerifyReportV1` (verification executed)

```json
{
  "akmon_version": "2.0.0",
  "agef_version": "0.1.1",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "journal_path": "/home/alice/.local/state/akmon/journal",
  "events_checked": 14,
  "objects_checked": 31,
  "passed": false,
  "checks_performed": [
    "parent_chain",
    "sequence",
    "event_hash_recompute",
    "object_presence",
    "object_byte_rehash",
    "head_consistency",
    "session_end_invariants"
  ],
  "violations": [
    {
      "category": "object_hash_mismatch",
      "event_hash": null,
      "object_hash": "8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757",
      "message": "Object bytes do not match hash"
    }
  ]
}
```

Field meanings:

- `akmon_version`: Akmon CLI version that produced this report.
- `agef_version`: AGEF specification version implemented by this verifier (`akmon-journal::AGEF_SPEC_VERSION`).
- `session_id`: session UUID (hyphenated lowercase).
- `journal_path`: resolved absolute journal directory path used.
- `events_checked`: total events walked.
- `objects_checked`: total referenced object checks performed.
- `passed`: true when no violations were found.
- `checks_performed`: stable list of attempted verification checks (snake_case).
- `violations`: array of violation objects.

Violation fields:

- `category`: stable category identifier.
- `event_hash`: optional event hash in lowercase hex.
- `object_hash`: optional object hash in lowercase hex.
- `message`: human-readable detail string.

Known violation categories:

- `missing_object`
- `object_hash_mismatch`
- `event_hash_mismatch`
- `parent_chain`
- `sequence`
- `head_mismatch`
- `session_end_missing`
- `session_end_duplicate`
- `session_end_not_terminal`

#### `VerifyError` (verification could not run)

```json
{
  "akmon_version": "2.0.0",
  "category": "session_not_found",
  "error": "cannot open journal /tmp/my-journal for session 550e8400-e29b-41d4-a716-446655440000: session not found: 550e8400-e29b-41d4-a716-446655440000"
}
```

Field meanings:

- `akmon_version`: Akmon CLI version that produced this error.
- `category`: infrastructure error class:
  - `journal_not_found`
  - `session_not_found`
  - `verify_infrastructure_error`
- `error`: human-readable diagnostic message.

## What verify checks (AGEF Section 13 alignment)

- **Parent chain integrity**: each non-start event points to the expected prior event.
- **Sequence integrity**: event sequences are contiguous (`0..n-1`).
- **Event hash recompute**: canonical CBOR event bytes hash to stored event hashes.
- **Object presence**: each referenced object hash resolves in object storage.
- **Object byte re-hash**: resolved object bytes hash back to referenced object hashes.
- **Head consistency**: stored session head equals the terminal event hash.
- **SessionEnd invariants**: exactly one `SessionEnd`, and it is terminal.

For formal semantics, see AGEF specification Section 13: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef).

## What verify does not check

- Cross-session consistency or relationships between sessions.
- Timestamp chronology correctness versus wall clock or between events.
- Semantic correctness of model/tool decisions.
- Bundle/manifest version compatibility checks (this command verifies substrate journal sessions, not AGEF bundle manifests).

## Worked examples

### 1) Verify a clean session

```bash
$ akmon verify 550e8400-e29b-41d4-a716-446655440000
verified: session 550e8400-e29b-41d4-a716-446655440000
  events checked: 14
  objects checked: 31
  SessionEnd: present and terminal

$ echo $?
0
```

### 2) Verify a corrupted session

```bash
$ akmon verify 550e8400-e29b-41d4-a716-446655440000
verification failed: session 550e8400-e29b-41d4-a716-446655440000
  events checked: 14
  objects checked: 31

  violations:
    - object hash mismatches: 1
    - ...

$ echo $?
1
```

### 3) JSON output for CI

```bash
$ akmon verify 550e8400-e29b-41d4-a716-446655440000 --format json | jq '.passed'
true
```

### 4) Verbose detail

```bash
$ akmon verify 550e8400-e29b-41d4-a716-446655440000 --verbose
verified: session 550e8400-e29b-41d4-a716-446655440000
  events checked: 14
  objects checked: 31
  SessionEnd: present and terminal

  checks performed:
    - parent chain: ok
    - sequence: ok
    - event hash recompute: ok
    - object presence: ok (31)
    - object byte re-hash: ok (31)
    - head consistency: ok
```

### 5) Custom journal path

```bash
$ akmon verify 550e8400-e29b-41d4-a716-446655440000 --journal /tmp/my-journal
```

## Programmatic / CI usage

- Use `--format json` and parse `passed`, `violations`, and `checks_performed`.
- Use exit codes for build pass/fail gates (`0` vs `1`/`3`).
- Recommended pattern: verify every session artifact produced in automated test runs.
- Example CI shell pattern:

```bash
akmon verify "$SESSION_ID" --format json > verify.json
jq -e '.passed == true' verify.json
```

## See also

- AGEF specification: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef)
- `akmon inspect` (Item 4.2, coming)
- `akmon export` / `akmon import` (Item 4.3, coming)
