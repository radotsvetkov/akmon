# Akmon

Akmon is a **local-first, trust-first** terminal agent for working inside a git-backed project. It is aimed at developers who want a single **Rust-native** binary that can reason over their tree with explicit permissions, a hard iteration ceiling, and a **complete audit trail** of every policy decision and tool step. Unlike opaque cloud assistants, Akmon keeps the model, tools, and filesystem boundary under your control: it talks to **Ollama** on your machine or to the **Anthropic API** when you provide a key, and it writes a **full JSONL audit log** of everything it did so runs stay accountable and reproducible.

## Install

### From source

Prerequisites: Rust toolchain via [rustup](https://rustup.rs/).

```bash
git clone https://github.com/your-org/akmon
cd akmon
cargo build --release
# binary at target/release/akmon
# optionally: cp target/release/akmon /usr/local/bin/
```

### Quick start with Ollama

```bash
ollama pull llama3.2
akmon --yes --task "describe this codebase"
```

### Quick start with Anthropic

```bash
export ANTHROPIC_API_KEY=your_key
akmon --yes \
  --model claude-haiku-4-5 \
  --task "describe this codebase"
```

## How it works

Akmon runs a tight **agent loop** backed by an explicit finite-state machine. Each `--task` turn starts from `Idle`, moves through planning and thinking, and can cycle through tool execution and optional confirmation gates until the model ends the turn or a **hard iteration limit** stops the loop. Tool results and streamed assistant text are folded back into **multi-turn context** so the model can continue with full visibility into what it already tried.

The **sandbox** is rooted at your **git project root** (Akmon walks upward from the current directory to find `.git`). Every filesystem tool path is resolved and validated against that root: paths are normalized, **symlinks are resolved before** the boundary check, and attempts to escape outside the tree are rejected.

The **policy engine** runs in one of several modes (deny-all, interactive, auto-approve reads with optional write confirmation, or auto-approve reads plus optional web fetch when `--yes-web` is used with `--web-fetch`). **Every** permission check produces an audit record with **verdict and reason**, so you can see not only what happened but why it was allowed or denied.

The **audit log** is a **JSON Lines** file (one JSON object per line) written under `.akmon/audit/` by default, named after the session UUID. When you use `--output json`, the printed `RunReport` includes the same `session_id` and `audit_log_path`, so machine-readable stdout and the on-disk audit file stay linked.

## Available tools

| Tool | What it does | Permission required |
|------|----------------|---------------------|
| `list_directory` | List files and subdirectories in a path | `ListDirectory` (auto-approved with `--yes`) |
| `read_file` | Read a UTF-8 text file | `ReadFile` (auto-approved with `--yes`) |
| `write_file` | Write content to a file atomically | `WriteFile` (always requires interactive confirmation) |
| `shell` | Run an allowlisted argv-only subprocess (no shell interpreter) | `ExecuteCommand` (always requires interactive confirmation; opt-in via `--shell-allow`) |

### MCP tools (`--mcp-server`)

Connect Akmon to any MCP server to extend it with additional tools:

```bash
akmon --mcp-server http://localhost:3000 \
  --task "use the database tool to..."
```

Tools are discovered automatically at startup via the MCP protocol (`tools/list`); each invocation uses `tools/call` over JSON-RPC. MCP tools require **`NetworkFetch`** permission to the server base URL (same policy rules as `web_fetch` where applicable).

## CLI reference

| Flag | Default | Description |
|------|---------|-------------|
| `--task` / `-t` | none | Task to run |
| `--model` | `llama3.2` | Model name |
| `--ollama-url` | `http://localhost:11434` | Ollama base URL |
| `--anthropic-key` | env `ANTHROPIC_API_KEY` | Anthropic API key |
| `--yes` / `-y` | false | Auto-approve read-only operations (`ReadFile`, `ListDirectory`) |
| `--yes-web` | false | With `--yes` and `--web-fetch`, auto-approve `NetworkFetch` to URLs that pass tool-side SSRF checks |
| `--output` | `text` | Output format: `text` or `json` |
| `--audit-log` | `.akmon/audit/{session_id}.jsonl` | Audit log file path |
| `--shell-allow` | _(none)_ | Glob pattern for an allowlisted argv-only `shell` tool command; repeatable (see trust model) |
| `--web-fetch` | false | Register the `web_fetch` tool (off by default; see trust model) |
| `--mcp-server` | _(none)_ | MCP server base URL; register all tools from that server (repeatable) |

## AKMON.md — project memory

`AKMON.md` is an **optional** file at the **project root**. When present, Akmon loads it at session start and injects it as **project context** alongside the rest of the prompt. It is meant to be **user-curated** and **version-controlled**: describe conventions, layout, and how you want the agent to behave. Akmon does **not** write or modify `AKMON.md` on its own; any change to that file should be something you explicitly approve outside the tool.

Minimal example:

```markdown
# My Project

## Structure

- `src/` contains the main application
- `tests/` contains integration tests

## Conventions

- Use snake_case for all identifiers
- Every public function needs a doc comment
```

## Trust model

With **`--yes`**, Akmon pre-approves only **read-only** filesystem checks: **`ReadFile`** and **`ListDirectory`**. It does **not** auto-approve **`WriteFile`**, **`ExecuteCommand`** (the `shell` tool), or **`NetworkFetch`** (`web_fetch`) unless you opt in further. **`WriteFile` and `shell` always require an explicit confirmation step** in the current design, even when **`--yes`** is set.

When you enable **`--web-fetch`** and want headless runs to fetch public documentation without a prompt, add **`--yes-web`** alongside **`--yes`**. That switches the policy engine to **auto-approve reads and fetch**: **`NetworkFetch`** is allowed automatically after the tool’s SSRF validation (private IPs, metadata endpoints, and other blocked targets are still rejected before any request). **`WriteFile`** and **`shell`** behavior is unchanged—they still require confirmation. If you use **`--yes`** and **`--web-fetch`** but **not** **`--yes-web`**, each fetch still goes through the normal confirmation prompt.

The `shell` tool is only registered when you pass at least one **`--shell-allow <PATTERN>`**; commands must match your allowlist and are executed as a plain argv split (no shell interpreter). Every policy decision is written to the audit log with a **reason**. To inspect a run, use `cat .akmon/audit/*.jsonl` or pass **`--output json`** to get a session summary on stdout that points at the same audit file via `session_id` and `audit_log_path`.

## Security

All tool paths are confined by the **sandbox**: resolution starts from the **git root**, uses canonical paths, and enforces a **prefix check** so content outside the project tree is unreachable. **Path traversal** and **`..` escapes** are rejected, and **symlinks are resolved before** comparing against the allowed root so indirect escapes fail closed.

**API keys** are held in a **`Secret`** type that **does not implement `Debug`**, **zeroizes on drop**, and is read only through **`expose_secret()`** at controlled call sites (for example building HTTP headers), so secrets should not appear in logs or structured debug output.

**Prompt injection** from disk is mitigated by **structural delimiters** around injected blocks (project context, `AKMON.md`, tool outputs): file contents are framed as **data**, not as trusted system instructions, alongside a fixed system role for the agent.

## License

Licensed under **MIT OR Apache-2.0** at your option — the standard Rust ecosystem dual license.

See `LICENSE-MIT` and `LICENSE-APACHE` at the repository root.
