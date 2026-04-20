# Tools reference

Akmon’s agent invokes **tools** the model chooses from a fixed registry. Availability depends on **mode** (for example plan mode registers read-only tools) and **CLI flags** (`--web-fetch`, `--index`, `--shell-allow`, …).

## Categories

### Read & navigate

- **read_file** — read a file inside the sandbox.
- **list_directory** — list directory entries.
- **search** — ripgrep-style content search.

### Edit

- **write_file** — create/overwrite (with confirmation and diff preview).
- **edit** / patch-style tools — apply targeted edits (with confirmation where configured).

#### Dry-run diff preview (`file_change_set`)

File-modifying tools support a deterministic diff payload that can be inspected before writes:

- `patch` and `apply_patch` support `dry_run: true`.
- `write_file` and `edit` also support `dry_run: true` for preview-first workflows.
- When `dry_run` is `true`, tools still run full validation and diff generation, but skip disk mutation.

`file_change_set` success payload shape:

- `type`: always `file_change_set`
- `mode`: `applied` or `dry_run`
- `changes[]`: canonical per-file diff entries (`path`, `diff`, `lines_added`, `lines_removed`, `lines_changed`)
- `summary`: aggregate line/file counts
- `risk`: heuristic risk classification (`low`, `medium`, `high`)
- `files[]`: backward-compatible alias of `changes[]`

Practical flow:

1. Run `patch` (or `apply_patch`) with `dry_run: true`.
2. Parse `summary` + `risk` and inspect each `changes[i].diff`.
3. Re-run the same call without `dry_run` to persist changes (`mode: "applied"`).

### Git

- **git** — status, diff, log, add, commit, etc. (see [Git integration](../features/git.md)).

### Network

- **web_fetch** — HTTPS fetch with SSRF protections (optional via flag).

### Semantic

- **semantic_search** — embedding search when `--index` and full build (see [Semantic search](../features/semantic-search.md)).

### MCP

Dynamic tools from configured MCP servers ([MCP](../features/mcp.md)).

## Permissions

The [security model](../features/security.md) and policy engine decide auto-approval vs confirmation. Writes and dangerous operations require explicit approval unless your mode says otherwise.

## Scout workflow (read-only)

`akmon scout` is a bounded read-only analysis workflow that generates a `context_scout.v1` dossier under `.akmon/context/`.

- Uses read signals only (filesystem listing/reading/search-like path analysis).
- Does not invoke write/edit/patch/apply_patch/shell side effects.
- Emits deterministic sorted sections (`scanned_paths`, `key_entrypoints`, `candidate_files`, `related_tests`) and explicit truncation indicators when bounds are hit.

## Schema

Each tool exposes a **JSON Schema** for arguments; the model must call with valid JSON. Errors and outputs are fed back to the model and logged to the [audit log](../features/audit-log.md).
