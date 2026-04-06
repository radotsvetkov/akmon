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

## Schema

Each tool exposes a **JSON Schema** for arguments; the model must call with valid JSON. Errors and outputs are fed back to the model and logged to the [audit log](../features/audit-log.md).
