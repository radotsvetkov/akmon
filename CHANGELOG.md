# Changelog

All notable changes to Akmon are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.7.7] - 2026-04-10

### Added

- **TUI `/config` and Ctrl+S:** full-screen **settings** overlay with an **Estimates** tab to edit **`[[model_estimates]]`** for the current model (context window tokens, optional USD per 1M input/output/cache-read, note). Saves to `~/.akmon/config.toml` and reloads in-session estimates for the agent.
- **Configurable model estimates:** `[[model_estimates]]` in user config for context-window % and rough USD cost; documented in getting started and configuration reference.

### Changed

- **Cost estimate behavior:** `free_local` / Ollama-style sessions return **$0** without requiring a built-in pricing row for the model id.
- **Documentation:** clarifies that context **window %** is separate from provider **rate limits**; cost display is explicitly a rough estimate. README links TUI settings to cost transparency.

### Fixed

- **Docs:** `[[model_estimates]]` TOML examples and reference table use the correct field names (`input_per_million_usd`, etc.).

## [1.7.6] - 2026-04-09

### Added

- **`akmon config` wizard:** running `akmon config` with no subcommand starts an interactive stdin flow (default model, optional Anthropic/OpenRouter keys, Ollama URL). `akmon config --json` still requires an explicit subcommand.
- **TUI `/transcript`:** exports the current chat to `.akmon/transcript_export.md` for reading outside the alternate-screen UI.
- **TUI `/mcp`:** scrollable panel with `akmon config mcp …` recipes and configured MCP servers from `~/.akmon/config.toml`.
- **TUI `/view-plan`:** full plan content in a scrollable overlay (with **PgUp/PgDn** on audit-style overlays).

### Changed

- **TUI `/resume`:** bare `/resume` shows usage; **`/sessions`** remains the session picker (no duplicate behavior).
- **Permission dialog:** clearer labels for once vs “remember for session” (exact permission match) vs broad allow.
- **Documentation:** configuration page covers wizard behavior, scrollback limits, and env/TOML; env-vars page adds wizard vs env notes.

## [1.7.5] - 2026-04-10

### Fixed

- **TUI context usage:** context bar and `/context` percent now include cumulative cache-read tokens so usage matches provider-reported prompt pressure (for example Anthropic with heavy caching).

### Changed

- **TUI usability:** mouse capture defaults off so native mouse/trackpad text selection works without toggling; **Ctrl+M** still enables wheel scrolling.
- **TUI transcript:** inline colored diff preview for `file_edit_diff` tool results before expanding with Tab.
- **README:** restored anvil header art, passed-tests badge, “what Akmon means” footer, and clarified the live-session example wording.

## [1.7.4] - 2026-04-09

### Changed

- **Documentation rewrite:** expanded core docs depth across `README.md` and mdBook with practical, production-style guidance for usage, architecture, MCP, costs, headless automation, and contributor internals.
- **Operational guides:** added stronger troubleshooting and common-mistakes sections across comparison, security, git, semantic search, interactive mode, planning modes, and capabilities reference.

## [1.7.3] - 2026-04-09

### Fixed

- **Rate-limit handling:** avoid re-entering the outer session loop after provider-level `RateLimited`, including summarization paths, so exhausted retries surface cleanly.
- **Retry UX consistency:** preserve provider-owned Anthropic retry countdown semantics and prevent session-level swallowing of terminal rate-limit errors.
- **TUI usability:** added `/copy` to copy the latest assistant response to clipboard (with `.akmon/last_response.txt` fallback) and allowed `Shift+drag` native terminal selection passthrough.
- **Ollama resiliency:** added model-size-aware stream idle timeouts and aggressive context trimming for local models with an explicit status hint.
- **Session restore accounting:** resume now restores persisted cumulative token totals instead of resetting counters.

## [1.7.2] - 2026-04-09

### Changed

- **Token efficiency:** reduced global system prompt verbosity and removed redundant prompt sections to lower per-turn context cost.
- **Tool reference:** replaced long tool reference content with concise, high-signal descriptions to reduce recurring token overhead.
- **TUI:** added `/context` command to show context-window usage, estimated breakdown, and compact headroom.

### Fixed

- **Prompt assembly:** removed stale `OUTPUT_BREVITY` export and aligned tests with token-efficiency targets.
- **Context UX state:** track `AKMON.md` and specs presence in TUI app state for context diagnostics.

## [1.7.1] - 2026-04-09

### Fixed

- **Todo persistence across `-c` / `--continue`:** todo storage is now project-scoped at `.akmon/todos/current.json` instead of session-id filenames, so active tasks survive resumed sessions.
- **Todo prompt injection:** active task loading now reads the project-level todo file and no longer depends on `session_id`.
- **Todo lifecycle cleanup:** when all tasks are completed, `current.json` is removed automatically to avoid stale completed-only todo context.

## [1.7.0] - 2026-04-08

### Added

- **Documentation:** tutorials (step-by-step for Rust, Go, Python Flask/FastAPI, Elixir), multi-agent/automation patterns, architecture trade-offs; **capabilities** reference page; new examples for Flask/FastAPI and Elixir/Phoenix.
- **Site:** landing page refresh (live demo preview, community links) and book cross-links.

### Changed

- **License:** Apache-2.0 **only** (MIT option removed). Full text in repository `LICENSE`.
- **Provider resolution:** `LlmConnectConfig::resolve()` returns explicit `ProviderError` when a backend cannot be used (for example Claude-family models without API keys) instead of falling through to an unintended provider.
- **CLI:** with `--output json`, early configuration errors emit JSON on stdout for consistent automation parsing.

## [1.6.0] - 2026-04-08

### Added

- **Anthropic prompt caching**: multi-block system prompts with `cache_control`, tool-definition cache marker, and conversation cache hints; footer and cost logic surface cache read tokens.
- **TUI**: OSC 8 URL linkification in transcript text; `[display] theme = "light"` for readable body text on light terminals; status bar shows `tokens` / optional green `cache` with comma grouping.
- **Ollama**: loading-hint status messages while waiting for the first stream bytes; first-token timeouts tuned for local models.
- **Permissions**: session allowlist, allow-all-writes, and shell-prefix rules with labeled dialog options (`y` / `s` / `p` / `r` / `n`).
- **Exit summary**: ANSI-formatted session summary on stdout after the TUI closes.
- **`StreamEvent::StatusHint`**: propagated to `AgentEvent::StatusInfo` for provider UX hooks.

### Changed

- Provider label in the TUI follows confirmed backend after the first successful API response (`ProviderConfirmed`).
- Local models: optional reduced tool set for Ollama to cut prompt size.

## [1.5.1] - 2026-04-06

### Fixed

- **GitHub Releases** now ship prebuilt `akmon-darwin-arm64`, `akmon-darwin-x86_64`, and `akmon-linux-x86_64` binaries when you push a `v*` tag (the workflow previously created an empty release).
- **TUI compose box**: bracketed paste support, up to **512 KiB** of input, and no arbitrary **6-line** cap — large prompts and multi-line paste no longer truncate or break submission.
- **Project layout**: creating `.akmon/plans`, `.akmon/audit`, and `.akmon/specs` when launching the TUI or headless `--task` (skips seeding when the sandbox root is your home directory without a git repo, so global `~/.akmon` config is not confused with a project workspace).
- **Plan mode**: if writing `.akmon/plans/*.md` fails, the TUI now shows the error instead of failing silently.

### Added

- **Project intelligence layer** (`akmon-core::lang_profile`): language profiles (Rust, Python, TypeScript, JavaScript, Go, Java, C#, Elixir, Ruby, Swift, Kotlin, Dart/Flutter, C++, Zig), 40+ framework profiles (web, mobile, data, CLI/TUI, API specs), database and data-tool heuristics, architecture hints, and a capped (4000-byte) formatted block for prompts.
- Detection from manifests and bounded scans: `detect_language`, `detect_frameworks`, `detect_databases`, `detect_data_tools`, `detect_architecture_hints`, plus `build_project_profile` / `format_project_intelligence_for_root`.
- **Context injection**: the intelligence block is appended to `akmon init` project context and to the agent system prompt in `akmon-query` (normal and plan mode), before the tool reference.
- **TUI polish (Gemini-style)**: two-line status bar (short cwd, model, provider; session id, tokens, cache, estimated USD, step), optional context row for files touched this session, first-session and missing-`AKMON.md` welcome hints, plaintext exit summary after quit, `$EDITOR` breakout for `/edit-plan` and `/update-context`, and plan-save system lines with `/implement` / `/edit-plan` / `/view-plan`. Idle **Ctrl+C** exits the same way as **Ctrl+D** / `/exit`.

## [1.5.0] - 2026-04-06

### Added

- `akmon import`: read context files from Claude Code (`CLAUDE.md`), Cursor, Codex (`AGENTS.md`), Gemini CLI, Kiro, Windsurf, GitHub Copilot, Cline, Aider, and synthesize into `AKMON.md` using the configured model.
- `akmon export`: write `AKMON.md` content to any tool format — `claude-code`, `codex`, `cursor`, `gemini`, `kiro`, `copilot`, `windsurf`, `cline` (`--all` or `--tool <name>`).
- `/import` and `/export` TUI slash commands.
- Welcome screen detects existing tool context files and suggests `/import`.
- `akmon init` detects and offers to import existing context files.

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
