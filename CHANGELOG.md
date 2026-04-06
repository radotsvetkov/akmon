# Changelog

All notable changes to Akmon are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.4.0] - 2026-04-06

### Added

- Plan mode (`--plan` flag, `/plan` in TUI): read-only analysis that produces a written plan before any code is written. Write/edit/shell/git/MCP tools are not registered in plan mode.
- Architect mode (`--architect`, `--planner-model`, `[architect]` in `~/.akmon/config.toml`): two-phase workflow—planner model produces a plan, main model implements. Plan is saved under `.akmon/plans/`.
- Spec workflow (`akmon spec`): three-phase documents under `.akmon/specs/<feature>/` (`requirements.md` → `design.md` → `tasks.md`) plus `implement` for one unchecked task at a time (re-spawns the agent with forwarded CLI flags).
- TUI slash commands: `/plan`, `/implement`, `/architect`, `/spec`, `/update-context` (open `AKMON.md` in `$EDITOR` and reload).
- Improved `AKMON.md` generation template: Product, Architecture, Conventions, **Current sprint**, and Done sections for better steering across sessions.

### Changed

- MSRV raised to **1.88** (required by the `fastembed` dependency chain: `ort`, ICU crates).

## [1.3.0] - 2026-04-06

### Added

**TUI interactive mode**

- Full terminal UI with ratatui
- Streaming tokens rendered in place
- Tool call cards with expand/collapse
- Slash commands: `/help` `/clear` `/new` `/sessions` `/resume` `/model` `/mcp` `/index` `/audit` `/cost` `/exit`
- Session persistence and resume
- Syntax highlighted code blocks
- Pixel art Akmon anvil welcome screen
- Mouse click cursor positioning in input field
- `/model` picker showing installed Ollama models and Anthropic models
- `/mcp` panel with connection health
- Interrupt with Ctrl+C

**Project initialization**

- `akmon init`: detect project type and generate AKMON.md
- `akmon new`: scaffold new projects (Rust, Node, Python, Go)
- Sandbox works in non-git directories
- `/init` and `/new` slash commands in TUI

**GitTool**

- Native git status, diff, log, add, commit, branch, stash, show, restore as typed JSON outputs
- Auto-registered in git repos
- `--auto-commit` flag: each file edit becomes a reversible commit

**Config CLI**

- `~/.akmon/config.toml`: single TOML config file for all settings
- `akmon config model`: get/set/list/test
- `akmon config key`: manage API keys
- `akmon config mcp`: add/remove/enable/disable/test MCP servers
- `akmon config show`/`edit`/`reset`/`path`
- `--json` flag on all config commands

### Changed

- Project context now prioritizes `semantic_search` before `search` and `list_directory` for conceptual queries — dramatically reduces token usage per task
- Default Anthropic model: `claude-haiku-4-5-20251001`
- Sandbox allows cwd as root when no git repository found

### Performance

- `semantic_search` called first for conceptual queries: ~60% fewer tool calls per exploration task

## [1.2.0] - 2026-04-06

### Added

- Parallel tool execution: concurrent independent tool calls, results in original request order
- Anthropic prompt caching: ~93% token reduction on system context after first call (requires dated snapshot ID)
- Semantic repo indexing (`--index`): BGESmallENV15 embeddings, persisted to `.akmon/index.bin`
- SemanticSearchTool: natural language code search across the project
- `.gitignore`-aware indexer: respects existing ignore rules, skips `target/`, lock files, binaries automatically
- `.akmonignore` support for project-specific exclusions
- `max_files` cap (default 500) with clear warning message
- Progress reporting during index build
- `tool_reference.txt`: detailed tool documentation in system context
- `--yes-web` flag and `AutoApproveReadsAndFetch` policy mode

### Changed

- Default Anthropic model: `claude-haiku-4-5-20251001`
- fastembed upgraded to v5
- Index loads synchronously when `.akmon/index.bin` exists
- Indexer replaced walkdir with `ignore` crate for `.gitignore` support

### Performance

- Parallel tools: ~50% faster on multi-file tasks
- Prompt caching: ~93% system context cost reduction
- Index build: ~100x fewer files indexed due to `.gitignore` respect

### Fixed

- Index thread no longer dropped before save completes
- Indexer no longer scans generated files, lock files, and binaries

## [1.1.0] - 2026-04-05

### Added

- SearchTool: search files with regex, file pattern filter, context lines
- EditTool: surgical string replacement in files, exact match required
- PatchTool: apply unified diffs to one or more files
- Context summarization: automatic compression when approaching context window limit
- WebFetchTool: fetch public URLs with SSRF protection, opt-in via `--web-fetch`
- MCP client: connect to MCP servers via `--mcp-server`, auto-discover tools
- `--yes-web` flag: auto-approve web fetch (SSRF always enforced)
- AutoApproveReadsAndFetch policy mode

### Changed

- Rust edition updated to 2024
- MSRV set to 1.85
- Default Anthropic model updated to claude-haiku-4-5

### Security

- SSRF protection on all web fetch requests blocks localhost, RFC1918, link-local, and cloud metadata endpoints
- Web fetch opt-in by default

## [1.0.0] - 2026-04-05

### What Akmon is

Akmon is a local-first, trust-first Rust AI coding agent. It runs as a single binary with no runtime dependencies, works fully offline with Ollama, and connects to the Anthropic API for frontier model access. Every action is audited.

### Features in v1.0.0

**Two model backends**

- Ollama — local models, fully offline, no data leaves the machine
- Anthropic — Claude models via API (use dated snapshot ids for stable caching)
- Local backend preferred by default, explicit confirmation required before any remote API call

**Three file tools**

- list_directory — explore project structure safely
- read_file — read UTF-8 text files
- write_file — atomic writes with no partial file states

**Shell tool with allowlist**

- Only commands matching explicit glob patterns are permitted
- Shell metacharacters always rejected
- Never auto-approved, always confirmed
- Configurable timeout and output limit

**Policy engine**

- Three modes: DenyAll, Interactive, AutoApproveReads
- Every permission decision logged to the audit trail
- No bypass path exists

**Sandbox**

- Git root auto-detected as boundary
- All paths canonicalized before boundary check
- Symlinks resolved before check
- Path traversal attempts rejected with typed error

**Audit log**

- JSONL file per session in `.akmon/audit/`
- Every tool dispatch, policy decision, and agent event recorded
- Machine-readable, linkable to JSON output via `session_id`

**Project memory**

- Optional `AKMON.md` at project root
- Loaded as system context at session start
- Never written without explicit user approval
- Plain markdown, version-controllable

**Output modes**

- `text`: streaming tokens to terminal
- `json`: single RunReport object on stdout for CI and scripting

**Single binary**

- `cargo build --release` produces one ~5.4MB binary
- No runtime, no installer, no dependencies
- Rust 2024 edition, MSRV 1.85

### Security properties

- Secrets stored as `Secret<T>` — zeroized on drop, never in logs
- File contents isolated in prompts as data, never as instructions
- Prompt injection mitigated by structural delimiters
- API keys never appear in audit log or debug output

### Known limitations

- No TUI interactive mode — use `--task` for all sessions
- No Candle inference backend — local models require Ollama
- No Anthropic prompt caching — `AKMON.md` tokens consumed each turn
- Shell tool output interpreted by model, numeric results may be approximate

### CLI quick reference

```bash
# Local model
akmon --yes --task "describe this codebase"

# Anthropic
export ANTHROPIC_API_KEY=your_key
akmon --yes \
  --model claude-haiku-4-5-20251001 \
  --task "describe this codebase"

# With shell tool
akmon --yes \
  --shell-allow 'cargo test *' \
  --shell-allow 'git *' \
  --task "run the tests and summarize"

# CI / scripting
akmon --yes --output json \
  --task "..." | jq .result
```

### What is next (v1.1 targets)

- TUI interactive session
- Anthropic prompt caching for `AKMON.md`
- Candle pure-Rust inference backend
- Web fetch tool with SSRF protection
- Semantic repo indexing (RAG)
- Published crates.io release
