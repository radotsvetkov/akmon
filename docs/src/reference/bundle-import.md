# akmon bundle import

## Synopsis

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

## Description

`akmon bundle import` reads an `.akmon` bundle, validates it against AGEF integrity rules, and either reports validation results (`--verify-only`) or ingests the session into a local journal (default mode).

Use `--verify-only` when reviewing/auditing a bundle without mutating local state. Use default ingestion mode when you want to replay or adopt a portable session into local storage.

Ingestion mode writes to the target journal by design. `--verify-only` is the safe read-only mode for inspection pipelines and CI checks.

## Arguments

### `<bundle-path>` (required)

Path to an `.akmon` bundle file.

## Options

### `--journal <path>` (optional)

Target journal directory. If omitted, Akmon resolves the default D-04 journal path (`$XDG_STATE_HOME/akmon/journal`).

If the directory or `journal.redb` does not exist, import creates them.

### `--format <human|json>` (optional, default: `human`)

Status output format:

- `human`: terminal-oriented status/errors
- `json`: machine-readable report/error objects

### `--verify-only` (optional)

Validate the bundle without writing objects/events to a local journal.

### `--allow-extra-files` (optional)

Accept bundles containing files outside the AGEF normative set (`manifest.json`, `events.bin`, `objects/<hex>`). Default behavior is strict reject of unknown archive entries.

### `--rename-to <NEW_UUID>` (optional)

Import under a different session UUID than the bundle manifest session id. Use when the target journal already contains the bundle session id (collision).

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Bundle imported successfully (or verified successfully with `--verify-only`) |
| `1` | Bundle validation failed (AGEF integrity/structure violation) |
| `2` | Usage/recoverable import error (for example, session collision without suitable `--rename-to`) |
| `3` | I/O or environment error (bundle not found, unwritable journal, local store corruption) |

## Output formats

### Human (default, ingestion success)

```text
imported bundle: /path/to/audit.akmon
  original session: 550e8400-e29b-41d4-a716-446655440000
  imported as: 550e8400-e29b-41d4-a716-446655440000 (same as original)
  events: 14
  objects: 31 (28 new, 3 existing in store)
  target journal: /home/user/.local/state/akmon/journal
```

With `--rename-to`, `imported as` is reported as renamed.

### Human (`--verify-only` success)

```text
verified bundle: /path/to/audit.akmon
  session_id: 550e8400-e29b-41d4-a716-446655440000
  events: 14
  objects: 31
```

### Human (collision)

```text
akmon: bundle import: error: target journal already contains session 550e8400-e29b-41d4-a716-446655440000
akmon: bundle import: hint: use --rename-to <NEW_UUID> to import as a different session
```

Exit code is `2`.

### JSON (`--format json`)

#### `BundleImportReportV1` (ingestion success)

```json
{
  "akmon_version": "1.8.2",
  "agef_version": "0.1.1",
  "bundle_path": "/path/to/audit.akmon",
  "original_session_id": "550e8400-e29b-41d4-a716-446655440000",
  "imported_session_id": "7c9a2c4e-3f2c-4dbe-b58d-4e9d0e1c6a20",
  "events_imported": 14,
  "objects_total": 31,
  "objects_new": 28,
  "objects_existing": 3,
  "journal_path": "/home/user/.local/state/akmon/journal"
}
```

#### `BundleVerifyReportV1` (`--verify-only`)

```json
{
  "akmon_version": "1.8.2",
  "agef_version": "0.1.1",
  "bundle_path": "/path/to/audit.akmon",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "events_in_bundle": 14,
  "objects_in_bundle": 31,
  "passed": true,
  "violations": []
}
```

#### `BundleViolation`

```json
{
  "category": "object_key_hash_mismatch",
  "event_hash": null,
  "object_hash": "8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757",
  "message": "object bytes do not match object path digest"
}
```

Fields:

- `category`: stable violation category
- `event_hash`: optional event hash context
- `object_hash`: optional object hash context
- `message`: human-readable detail

#### `BundleImportInfraError`

```json
{
  "akmon_version": "1.8.2",
  "error": "target journal already contains session 550e8400-e29b-41d4-a716-446655440000",
  "category": "session_id_collision",
  "colliding_session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

`colliding_session_id` is present only for collision errors.

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

For normative definitions, see AGEF Section 13 and Section 14: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef).

## What bundle import does NOT do

- Does **not** merge sessions. Each import writes one new session row keyed by target UUID.
- Does **not** modify the bundle file (input is read-only).
- Does **not** guarantee byte-identical re-export outputs after import. Event hashes are preserved; archive byte layout can differ on re-export.
- Does **not** support partial event import on validation failure.
- Does **not** decrypt or verify signatures (hash-chain integrity only).

## Programmatic usage

- Use `--format json` + `jq`/parser for stable automation.
- Use `--verify-only` in CI before ingestion.
- Treat exit `2` collisions as recoverable via `--rename-to`; investigate exits `1` and `3`.

## Examples

### 1) Import into default journal

```bash
$ akmon bundle import audit.akmon
```

### 2) Verify-only (no writes)

```bash
$ akmon bundle import audit.akmon --verify-only
```

### 3) Import with rename to avoid collision

```bash
$ akmon bundle import audit.akmon --rename-to 7c9a2c4e-3f2c-4dbe-b58d-4e9d0e1c6a20
```

### 4) Import into custom journal path

```bash
$ akmon bundle import audit.akmon --journal ~/audit-journals
```

### 5) JSON verify-only in CI

```bash
$ akmon bundle import audit.akmon --verify-only --format json | jq '.passed'
```

### 6) Accept non-normative extra files

```bash
$ akmon bundle import vendor-bundle.akmon --allow-extra-files
```

## See also

- `akmon bundle export`: [./bundle-export.md](./bundle-export.md)
- `akmon verify`: [./verify.md](./verify.md)
- `akmon inspect`: [./inspect.md](./inspect.md)
- AGEF specification: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef)
