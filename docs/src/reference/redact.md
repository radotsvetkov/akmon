# akmon redact

## Synopsis

```bash
akmon redact <session-id> [OPTIONS]
```

```bash
akmon redact <session-id> \
  --output <path> \
  --object <hash> [--object <hash> ...] \
  --reason <text> \
  [--journal <path>] \
  [--format <human|json>]
```

## Description

`akmon redact` reads one session from an on-disk Akmon journal and writes a sanitized derivative `.akmon` bundle in which named objects are replaced by Akmon redaction sentinel markers. The source journal is read-only and is never modified by this command.

Use redaction when audit artifacts must be shared without leaking sensitive content (for example PII, credentials, or trade secrets). It is also appropriate for compliance-oriented workflows where a producer must distribute a sanitized derivative while preserving verifiable evidence structure.

Redaction is one-way by design. Sentinel objects in the derivative bundle do not contain original content bytes. If the source journal is destroyed (or otherwise unavailable), the redacted content is unrecoverable. There is no `unredact` operation.

## Arguments

### `<session-id>` (required)

Hyphenated UUID of the source session in the journal (`AgentSession` construction ID).

## Options

### `--output <path>` (required)

Path where the derivative bundle will be written. There is no default output path; redaction requires an explicit destination. The output file must not already exist.

### `--object <hash>` (required, repeatable)

Object hash (lowercase hex) to redact. Supply multiple `--object` flags to redact multiple objects in one invocation.

Each hash must:

- parse under the source journal hash algorithm, and
- be referenced by at least one event in the source session.

Unknown or invalid hashes are usage errors.

### `--reason <text>` (required)

Audit rationale written into each sentinel marker. There is no default; every redaction requires explicit reason text. This text is preserved in the derivative bundle and visible to downstream inspectors.

### `--journal <path>` (optional)

Source journal directory. If omitted, Akmon resolves the default D-04 path (`$XDG_STATE_HOME/akmon/journal`).

### `--format <human|json>` (optional, default: `human`)

Status output format:

- `human`: terminal-oriented summary
- `json`: machine-readable report/error object

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Derivative bundle written successfully |
| `1` | Reserved (not currently emitted by `redact`) |
| `2` | Usage error (output exists, invalid hash format, object not in session, missing required flag) |
| `3` | I/O or environment error (journal/session not found, write failure, unreadable referenced object) |

`akmon redact` has no current "validation failed" mode distinct from usage/environment errors, so it does not emit exit code `1`.

## Output formats

### Human (default)

```text
redacted: source session 550e8400-e29b-41d4-a716-446655440000
  events rewritten: 11
  objects redacted: 2
  source head: 8b2a3f7c...
  new head: 4e1d92a8...
  bundle: /path/to/sanitized.akmon
  size: 2.4 MB
```

### JSON (`--format json`)

#### `RedactReportV1`

```json
{
  "akmon_version": "1.8.2",
  "agef_version": "0.1.1",
  "source_session_id": "550e8400-e29b-41d4-a716-446655440000",
  "source_head": "8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757",
  "derivative_head": "4e1d92a8d43a5f9bf4905fd9578c2f67a6198e7f2db16e89f5a9f3245d4f49fd",
  "events_in_session": 14,
  "events_rewritten_count": 11,
  "objects_redacted_count": 2,
  "redacted_objects": [
    {
      "original_hash": "8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757",
      "sentinel_hash": "4e1d92a8d43a5f9bf4905fd9578c2f67a6198e7f2db16e89f5a9f3245d4f49fd",
      "original_size": 1024
    }
  ],
  "output_path": "/path/to/sanitized.akmon",
  "bundle_size_bytes": 2456789
}
```

Field meanings:

- `akmon_version`, `agef_version`: producing Akmon/AGEF versions.
- `source_session_id`: source session UUID.
- `source_head`, `derivative_head`: source and rewritten terminal event hashes.
- `events_in_session`: total events in source/derivative session.
- `events_rewritten_count`: events whose content hash changed due to redaction rewrite cascade.
- `objects_redacted_count`: number of redacted objects.
- `redacted_objects`: per-object mapping entries.
- `output_path`: resolved path of written bundle.
- `bundle_size_bytes`: final derivative bundle size.

#### `RedactedObjectEntry`

- `original_hash`: original object hash.
- `sentinel_hash`: replacement sentinel object hash.
- `original_size`: original object byte size.

#### `RedactError`

```json
{
  "akmon_version": "1.8.2",
  "error": "invalid --object hash 'zz-not-hex'",
  "category": "invalid_object_hash",
  "invalid_object_hash": "zz-not-hex"
}
```

Fields:

- `akmon_version`, `error`, `category`
- optional context fields by category:
  - `invalid_object_hash` when `category == invalid_object_hash`
  - `missing_object_hash` when `category == object_not_in_session`

## Sentinel format

Redacted objects are replaced by canonical-CBOR sentinel objects with this payload:

```json
{
  "akmon_redacted": true,
  "original_hash": "<hex of original>",
  "original_size": 1024,
  "reason": "<text from --reason>",
  "redacted_at": "<RFC3339 timestamp>"
}
```

This sentinel format is Akmon-specific. Sentinel payloads are valid AGEF objects (canonical CBOR + content-addressed), but the `akmon_redacted` marker convention is not part of AGEF v0.1.1. Future AGEF versions may standardize redaction sentinel semantics.

## Examples

### 1) Redact one object

```bash
$ akmon redact 550e8400-e29b-41d4-a716-446655440000 \
  --output sanitized.akmon \
  --object 8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757 \
  --reason "PII removal per GDPR request"
```

### 2) Redact multiple objects

```bash
$ akmon redact 550e8400-e29b-41d4-a716-446655440000 \
  --output sanitized.akmon \
  --object 8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757 \
  --object 4e1d92a8d43a5f9bf4905fd9578c2f67a6198e7f2db16e89f5a9f3245d4f49fd \
  --reason "Trade secret removal"
```

### 3) JSON output for scripts

```bash
$ akmon redact 550e8400-e29b-41d4-a716-446655440000 \
  --output out.akmon \
  --object 8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757 \
  --reason "PII" \
  --format json | jq '.objects_redacted_count'
```

### 4) Custom journal path

```bash
$ akmon redact 550e8400-e29b-41d4-a716-446655440000 \
  --journal /tmp/my-journal \
  --output out.akmon \
  --object 8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757 \
  --reason "test"
```

### 5) Verify derivative bundle before distribution

```bash
$ akmon redact 550e8400-e29b-41d4-a716-446655440000 \
  --output sanitized.akmon \
  --object 8b2a3f7c1ef0ea7e80f772f8f84f86b16f5527cd51ff8b0a464f157c4cd5c757 \
  --reason "compliance"
$ akmon bundle import sanitized.akmon --verify-only
```

## What redact does NOT do

- Does not modify the source journal. Source events and objects remain bit-identical.
- Does not perform field-level or span-level redaction (v2.0 supports whole-object redaction only).
- Does not support pattern/policy selectors for object targeting; all targets must be explicit `--object` hashes.
- Does not sign or encrypt derivative bundles.
- Does not preserve source head hash; derivative head changes because event hashes are rewritten from the substitution point forward.
- Does not provide reversibility. Sentinel payloads never include original object bytes.
- Does not verify source integrity before redaction. Run `akmon verify` first if pre-redaction integrity assurance is required.

## Workflow notes

- Use `akmon inspect --resolve` first to identify object hashes that correspond to content you need to redact.
- Use `akmon bundle import --verify-only` on the derivative before sharing externally.
- Derivative bundles preserve original session ID; importing into a journal that already has that ID requires `--rename-to`.

## See also

- [akmon verify](./verify.md)
- [akmon inspect](./inspect.md)
- [akmon bundle export](./bundle-export.md)
- [akmon bundle import](./bundle-import.md)
- AGEF specification: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef)
