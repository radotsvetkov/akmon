# akmon bundle export

## Synopsis

```bash
akmon bundle export <session-id> [OPTIONS]
```

```bash
akmon bundle export <session-id> \
  [--output <path>] \
  [--journal <path>] \
  [--format <human|json>]
```

## Description

`akmon bundle export` reads one session from an on-disk Akmon journal and packages it into a portable `.akmon` archive. The archive is an AGEF-aligned `tar.zst` bundle containing the session manifest, event stream, and referenced object bytes.

Use export when you need to hand off an audit artifact, archive completed session evidence, or share a session with another developer for replay/import. Export preserves semantic evidence (events, object hashes, parent chain) in a transportable form.

`akmon inspect` and `akmon verify` read local journals; `akmon bundle export` produces the portable equivalent of that journal slice. Internally, `.akmon` is `tar.zst`, so standard tar/zstd tooling can inspect the archive layout.

## Arguments

### `<session-id>` (required)

Hyphenated UUID of the source session in the journal.

## Options

### `--output <path>` (optional)

Output path for the bundle file. Default is `<session-id>.akmon` in the current working directory.

### `--journal <path>` (optional)

Source journal directory. If omitted, Akmon resolves the default D-04 journal path (`$XDG_STATE_HOME/akmon/journal`).

### `--format <human|json>` (optional, default: `human`)

Status output format:

- `human`: terminal-oriented summary
- `json`: machine-readable report/error object

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Bundle written successfully |
| `1` | Reserved (not currently emitted) |
| `2` | Usage error (for example, output path already exists) |
| `3` | I/O or environment error (journal/session not found, missing object in store, write failure) |

## Output formats

### Human (default)

```text
exported: session 550e8400-e29b-41d4-a716-446655440000
  events: 14
  objects: 31
  bundle: /path/to/output.akmon
  size: 2.4 MB
```

### JSON (`--format json`)

#### `BundleExportReportV1`

```json
{
  "akmon_version": "2.0.0",
  "agef_version": "0.1.1",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "output_path": "/path/to/output.akmon",
  "events_exported": 14,
  "objects_exported": 31,
  "bundle_size_bytes": 2456789
}
```

Field meanings:

- `akmon_version`, `agef_version`: producing Akmon/AGEF versions.
- `session_id`: exported session UUID.
- `output_path`: resolved path of the written bundle file.
- `events_exported`, `objects_exported`: counts included in the bundle.
- `bundle_size_bytes`: final on-disk bundle size in bytes.

#### `BundleExportError`

```json
{
  "akmon_version": "2.0.0",
  "error": "cannot open journal ...",
  "category": "session_not_found"
}
```

Field meanings:

- `akmon_version`: Akmon CLI version that produced this error.
- `error`: human-readable diagnostic string.
- `category`: stable error class (`output_exists`, `missing_object`, `malformed_journal`, `session_not_found`, `journal_not_found`, `bundle_error`, `io_error`).

## Bundle format

An `.akmon` bundle is a `tar.zst` archive containing:

- `manifest.json`
- `events.bin`
- `objects/<hex>`

For normative format details, see AGEF specification Section 6 and Section 13: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef).

## What bundle export does NOT do

- Does **not** verify the source session before exporting. Use `akmon verify`, or run `akmon bundle import --verify-only` on the exported bundle.
- Does **not** modify the source journal (read-only operation).
- Does **not** sign or encrypt bundles. Integrity comes from hash-linked evidence; signatures are future AGEF work.
- Does **not** guarantee byte-identical re-export outputs. Re-exports are semantically equivalent, but tar metadata/zstd encoding may differ.

## Examples

### 1) Default export to current directory

```bash
$ akmon bundle export 550e8400-e29b-41d4-a716-446655440000
```

### 2) Export to a specific path

```bash
$ akmon bundle export 550e8400-e29b-41d4-a716-446655440000 --output ~/audit/q3.akmon
```

### 3) JSON output for scripting

```bash
$ akmon bundle export 550e8400-e29b-41d4-a716-446655440000 --format json | jq '.bundle_size_bytes'
```

### 4) Export from a custom journal path

```bash
$ akmon bundle export 550e8400-e29b-41d4-a716-446655440000 --journal /tmp/my-journal
```

## See also

- `akmon bundle import`: [./bundle-import.md](./bundle-import.md)
- `akmon verify`: [./verify.md](./verify.md)
- `akmon inspect`: [./inspect.md](./inspect.md)
- AGEF specification: [github.com/radotsvetkov/agef](https://github.com/radotsvetkov/agef)
