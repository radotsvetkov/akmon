# Changelog

All notable changes to Akmon are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
- Anthropic — claude-haiku-4-5 and other Claude models via API
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
  --model claude-haiku-4-5 \
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
