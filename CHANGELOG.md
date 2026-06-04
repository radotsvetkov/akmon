# Changelog

All notable changes to Akmon are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.8.2] - 2026-04-20

### Added

- **Provider resolution explainability:** deterministic `ProviderResolutionTrace` (`selected_provider`, `selected_reason`, `model_id`, ordered `candidates[]` with `eligible`, `reason`, `missing_prerequisites`, `priority_order`) mirroring `LlmConnectConfig::resolve` without changing routing.
- **`akmon config explain-provider`:** prints the resolution trace (use `--json` on `config` or global `--output json` for machine-readable output). This is diagnostics only; it does not change which provider is selected at runtime.
- **`akmon doctor providers`:** embeds the same `provider_resolution` block in text and JSON reports for side-by-side troubleshooting with reachability checks.
- **Headless JSON runs:** `--output json` run summaries include an optional `provider_resolution` field with the trace for the effective CLI model and merged `~/.akmon/config.toml` (additive for automation).

### Notes

- Resolver priority and selection semantics are unchanged; all additions are strictly introspection/diagnostics. Traces never echo secret values—only named prerequisites (env vars / flags).

## [Unreleased]

### Added

- **`akmon sign`:** sign a session head via the configured signing hook (D-05); the hook is also auto-invoked after headless runs. Documents the path to wiring cosign or GPG ahead of native signing.
- **`akmon bundle verify <bundle>`:** verify an AGEF `.akmon` bundle's integrity — object re-hashing, hash-chain re-walk, and manifest head/count checks — without importing it, using the same store-independent verification path as `akmon bundle import --verify-only`.
- **Standalone `agef-verify` binary:** a minimal, separately distributable AGEF bundle verifier for auditors and CI, independent of the Akmon CLI, journal store, and agent runtime. Built and published alongside `akmon` in release artifacts.
- **Supply-chain CI:** `cargo-deny` gating (RustSec advisories, license policy, banned/duplicate crates, source checks) via `deny.toml`, a pinned Rust 1.88 toolchain, and release checksums plus SBOM generation.
- **Native session signing (AGEF v0.1.2, decision D-18):** turn a tamper-evident bundle into an *attributable* one. `akmon bundle sign <bundle> --key <pkcs8>` signs a domain-separated `AGEF-SIG-v1` statement over the session head with an offline Ed25519 key, appends the detached signature to `manifest.signatures[]`, and prints the signer's public key (hex) to distribute. Verify with `akmon bundle verify --verify-key <hex-file> [--require-signature]` or the standalone `agef-verify --verify-key`. Signatures never enter the merkle hash chain, and integrity verification stays independent of signature verification. Ed25519 via `ring` (already in the dependency tree) — no new supply-chain surface; OpenPGP was rejected because every pure-Rust implementation pulls the advisory-bearing `rsa` crate.

### Changed

- **`spawn_subagent` is gated off by default.** Multi-agent orchestration is an explicit non-goal of Akmon's thesis (decision document §1.2 / §3.4: "one agent, one session, one artifact"). The `spawn_subagent` tool is no longer registered in default sessions, and the agent prompt no longer references it. Set the `AKMON_EXPERIMENTAL_SUBAGENTS` environment variable to a truthy value (`1`, `true`, `yes`, or `on`) to opt the current process into the unsupported, experimental capability. This aligns the shipped tool surface with the locked thesis; the experimental flag is intentionally not part of `config.toml`.
- **AGEF spec version bumped to `0.1.2`** (from `0.1.1`) — an additive change for the optional `manifest.signatures[]` field. Compatibility is unchanged: version matching is major.minor, so `0.1.1` readers still accept `0.1.2` bundles and ignore unknown fields.

### Fixed

- Unify the AGEF specification version to a single source of truth (`AGEF_SPEC_VERSION`) across journal metadata, replay reports, and diff reports.
- Restore the `cargo fmt --all --check` CI gate after the `bundle_cmd` extraction introduced formatting drift.
- Restore the `cargo test --workspace --no-default-features` CI gate: bundle-subcommand parse tests (compiled only under `not(semantic-index)`) referenced fields made private by the `bundle_cmd` extraction and predated the `bundle verify` variant.

### Security

- Patch `rustls-webpki` TLS advisories (RUSTSEC-2026-0098, -0099, -0104).

## [2.1.0] - 2026-05-28

Stability and operator-experience release on top of the v2.0.0 evidence substrate. No AGEF, bundle-format, or `EventKind` changes.

### Added

- **Diff trust dry-run:** `patch`, `apply_patch`, `edit`, and `write_file` now support `dry_run` validation mode that computes full diffs without mutating files.
- **Context Scout Dossier:** new bounded `akmon scout` read-only workflow produces deterministic `context_scout.v1` JSON dossiers for planning/CI usage.
- **`first_token_deadline_ms` in `~/.akmon/config.toml`:** optional operator override for LLM first-token timeouts; applies after provider-specific defaults (including Ollama model heuristics).

### Fixed

- **Tool repeat-limit FSM:** emit `ToolCallCompleted { success: false }` from `Thinking` when the `read_file` / `list_directory` repeat guard fires, avoiding an `InvalidTransition` crash that exited the session with code 1 (reported in [#1](https://github.com/radotsvetkov/akmon/issues/1)).
- **Session resume:** reopen existing journal graphs on `-c` / `--continue-last` instead of failing with `session already exists`; skip duplicate `SessionStart` on resume; restore agent model context in the TUI.
- **Repeat-limit evidence:** align tool-result payload with FSM event (`success: false`) so metrics and model transcript match journal/UI.
- **Tool dispatch:** validate LLM tool arguments against each tool's JSON Schema before execution (including MCP proxies).
- **Mid-batch tool failures:** reset parallel-tool state and transition to `Failed` instead of leaving the session stuck in `ToolExecution`.
- **FSM drift:** handle `AwaitingConfirmation + Error(SessionFailed)` and `Summarizing + Error` in `next_state_after`.
- **Git tool:** sandbox-validate paths for `diff` / `log` / `show`; block repo-escape flags; resolve auto-commit paths through the sandbox.
- **HTTP clients:** remove timeout-less `reqwest::Client::new()` fallbacks in model backends.
- **Config load:** warn when `~/.akmon/config.toml` is invalid instead of silently using defaults.
- **Config wiring:** apply `default_model`, `ollama_url`, and enabled `[[mcp]]` entries from `~/.akmon/config.toml` (CLI flags still override non-default values).
- **Secrets in Debug:** redact API keys in `LlmConnectConfig` debug output.

### Changed

- **File diff payload contract:** file-modifying tools now return a stabilized `file_change_set` payload with explicit `type`, `mode` (`applied` or `dry_run`), canonical `changes[]`, aggregate `summary`, and `risk` classification.
- **Compatibility path:** `files[]` remains as a backward-compatible alias for existing parsers while `changes[]` is now canonical.
- **Dossier ingestion:** `--dossier <path>` injects validated scout context into subsequent implementation runs without adding a new orchestration subsystem.

## [2.0.0] - 2026-05-06

Akmon v2.0.0 is the production-ready release. Earlier 1.x releases were exploratory. The major version bump reflects formalization of the session evidence model, the AGEF v0.1.1 specification, and a substantially expanded command surface (`verify`, `inspect`, `bundle`, `redact`, `replay`, `diff`). Existing 1.8.x sessions remain readable but consumers integrating against session formats should pin to AGEF v0.1.1.

### Added

- **Content-addressed substrate foundation:** session graph and object store with merkle-linked integrity (`Item 1.2`), plus stronger provider-call invariants and head consistency checks.
- **AGEF v0.1.1 specification implementation:** Akmon v2.0.0 is the AGEF v0.1.1 reference implementation. Bundle format, content addressing, and session graph semantics conform to the published specification.
- **Provider and tool journaling primitives:** `JournalingProvider`/`AttemptObserver` instrumentation across backends and `JournalingTool` with input/output hashing.
- **Session-first event recording:** agent loop now records `SessionStart`, `UserTurn`, `PermissionGate`, and `AssistantTurn` events in the session journal.
- **`akmon verify`:** CLI verification command with human/JSON output and verbose diagnostics.
- **`akmon inspect`:** session inspection command with summary/verbose modes, JSON output, object resolution, and binary rendering modes.
- **`akmon bundle`:** export/import command family for portable session bundles, including `--verify-only` import path and ingestion flow.
- **`akmon redact`:** derivative-bundle redaction workflow with sentinel object support and inspect visibility for redaction sentinels.
- **Replay engine and `akmon replay`:** deterministic replay pipeline, strict/default comparison modes, persistence support, structured report output, and CLI command surface.
- **Diff engine and `akmon diff`:** structural and field-level session comparison engine and CLI command with output formatting and exit codes.
- **Resolve-mode content comparison for diff:** dereferenced object-content comparison support in diff workflows.
- **Documentation and references for new command surfaces:** release/runtime docs for verify, inspect, bundle, redact, replay, and diff commands.

### Changed

- **Release positioning:** README rewritten around regulated-engineering use cases and review-aware session evidence workflows.
- **Planning and release architecture docs:** v2 decision-document and phase planning records expanded to cover implemented release scope.
- **Workspace versioning:** workspace package version bumped from `1.8.2` to `2.0.0`.

### Fixed

- **Inspect UTF-8 truncation safety:** resolve preview now truncates lines at UTF-8 character boundaries, preventing panics when byte caps cut through multi-byte characters.
- **Replay engine robustness:** orchestration/channel-drain and comparison normalization fixes for deterministic replay behavior.
- **Diff integration reliability:** multi-session journal loading path improved in CLI integration coverage and behavior.

### Documentation

- **New command references:** replay/diff and other v2 command docs added or expanded under `docs/src/reference/`.
- **Release documentation updates:** release notes and planning artifacts aligned with v2 rollout.
- **Landing documentation refresh:** README and related docs now reflect regulated-engineering positioning and trust pipeline workflows.

## [1.8.1] - 2026-04-20

### Added

- **Provider operability diagnostics:** `akmon doctor providers` preflight checks with text/JSON output, actionable remediation hints, masked credential checks, endpoint sanity/reachability checks, and non-zero exits for critical provider failures.
- **Deterministic docs quality gates:** CI `docs-quality` job now enforces mdBook build, local markdown link checks, CLI snippet smoke checks, and JSON snippet sanity with fixture-based pass/fail validation.
- **Local reliability capability hints:** Ollama model metadata/probe hints are now used (when available) for timeout/context heuristics while preserving safe fallback behavior when probe data is unavailable.

### Changed

- **MCP governance posture:** MCP calls now consistently run through fail-closed server/tool policy evaluation (`mcp.servers`, `mcp.tools`) and emit enriched audit context (`mcp_server`, `mcp_tool`, `decision_reason`).
- **TUI maintainability (internal only):** `TuiApp` state was decomposed into focused internal modules (`composer`, `overlay_state`, `session_telemetry`, `provider_runtime`) with behavior parity and targeted transition tests; no user-facing UX/command changes.
- **Ollama status consistency:** streaming and buffered completion paths now share one status-hint scheduling flow for predictable local-model progress messaging.

### Fixed

- **Local timeout/remediation clarity:** first-token timeout, idle-stream timeout, missing-model, and no-output failure modes now return clearer operator guidance (`/clear`, `ollama ps`, warm model, switch model).
- **Local false-timeout reduction:** adaptive timeout floors for local models reduce cold-start false failures while keeping deterministic bounded behavior.

### Migration / Operator Notes

- No CLI surface or command semantics changes were introduced in this release.
- For configured-policy environments using MCP, define explicit MCP allow rules before production rollout; ambiguous/missing MCP context remains fail-closed by design.
- For local-model-heavy workflows, prefer warming models (`ollama run <model>`) before long tasks and use `akmon doctor providers` in CI/host readiness checks.

## [1.8.0] - 2026-04-20

### Added

- **Policy governance for enterprise environments:** built-in profiles (`dev`, `staging`, `prod`), composable local policy packs, deterministic merge precedence, and `akmon policy show-effective` introspection.
- **Policy-as-code runtime posture:** configured rule evaluation for filesystem/shell/network/tool access with deterministic deny/allow behavior.
- **Audit verification pipeline:** tamper-evident audit chain verification via `akmon audit verify`.
- **Replay/evidence trust linkage:** replay metadata in headless JSON output, evidence artifact generation/verification (`akmon evidence verify`), and deterministic policy-hash impact from effective merged policy.
- **Reliability guardrails:** run-level reliability metrics in run report/evidence, SLO verification (`akmon slo verify`), and baseline trend regression checks (`akmon slo trend`).
- **Operator docs and tutorials:** end-to-end local-first, CI governance, and enterprise policy-rollout tutorials plus release-focused trust pipeline guidance.

### Changed

- **Safety defaults hardening:** nested/subagent execution remains constrained by parent policy posture and fails closed for ambiguous side-effect contexts.
- **MCP governance hardening:** MCP execution now uses explicit fail-closed server/tool policy dimensions (`mcp.servers`, `mcp.tools`), denies ambiguous/malformed MCP context, and records enriched MCP audit context (`mcp_server`, `mcp_tool`, `decision_reason`).
- **Release/operator docs:** README, CLI/config/security/audit/evidence references, and landing copy now align to trust-runtime workflows and command behavior.
- **Versioning/package metadata:** workspace version advanced to `1.8.0`; docs and install examples updated accordingly.

### Migration notes

- **Audit consumers:** parse each JSONL line as `AuditChainRecord` (`schema_version: "audit_chain.v1"`), not raw `AuditEvent`.
- **Run report consumers:** treat `replay_metadata` and `reliability_metrics` as additive stable fields in `--output json`.
- **Evidence consumers:** require `evidence_schema_version` and validate linked audit/session hash consistency.
- **Policy governance rollout:** effective policy source order is explicit (`profile < packs < local < CLI override`); invalid selected pack inputs fail closed.
- **MCP operators:** define explicit MCP allow rules before using MCP in configured policy mode; unmanaged/ambiguous MCP context now denies by default.

### Operator impact

- CI can now gate runs with both integrity checks (`audit`/`evidence`) and reliability checks (`slo verify`/`slo trend`).
- Teams can stage policy hardening (`dev` -> `staging` -> `prod`) without changing permission classes.

## [1.7.7] - 2026-04-10

### Added

- **Audit verification CLI:** `akmon audit verify <path>` verifies audit-chain integrity and exits non-zero on invalid/tampered/unsupported files (supports `--output json`).
- **Replay metadata in run report:** headless `--output json` now includes a deterministic `replay_metadata` block (`hash_algorithm`, provider/model/session ids, policy/config/tool hashes, optional prompt assembly hash).
- **Evidence artifact:** headless runs now emit `.akmon/evidence/<session-id>.json` (`evidence.v1`) with replay metadata, audit linkage, policy summary, tool outcomes, and touched files. Add `--evidence-path` to override output location.
- **Evidence verification CLI:** `akmon evidence verify <path>` validates evidence schema and linked audit-chain integrity.
- **Reliability/SLO metrics:** headless run reports now include `reliability_metrics` (`tool_calls_*`, latency totals/avg/p95, `policy_denials_total`, `retries_total`, `timeouts_total`), and evidence artifacts include the same block for CI/ops consumption.
- **SLO guardrail command:** `akmon slo verify <path>` evaluates run-report/evidence reliability metrics against threshold policies (`[slo]` config defaults, threshold files, and CLI overrides) with CI-friendly non-zero exits on violations.
- **Trend regression detection:** `akmon slo trend <current-path>` compares current reliability metrics vs last-N baseline artifacts and fails on configurable degradation tolerances (`[slo.trend]` or `--config`), with structured JSON output for CI.
- **Enterprise policy profiles/packs:** built-in `dev`/`staging`/`prod` profiles, deterministic policy-pack loading, explicit merge precedence (`profile < packs < local < CLI override`), and `akmon policy show-effective` for operator introspection.

### Changed

- **Policy hardening:** dispatch-time policy checks now consistently use tool-context evaluation where tool names are known, preventing tool-rule bypass when permission class alone would allow.
- **Audit schema versioning:** each JSONL record now includes `schema_version: "audit_chain.v1"` for stable downstream parsing contracts.
- **Subagent safety defaults hardening:** nested `spawn_subagent` execution no longer injects broad pre-approved interactive allows; nested tool access is now capped by parent policy posture and fails closed when side-effect confirmations are ambiguous.
- **Configured policy assembly:** headless session policy mode now uses merged effective policy when profile/packs/local/override sources are present; no-source runs preserve prior interactive/`--yes` behavior.

### Migration notes

- **Audit consumers:** deserialize each line as `AuditChainRecord` instead of `AuditEvent`.
- **Event payload access:** read the original event via `.event` (flattened `event_kind` JSON remains present).
- **Schema validation:** expect and validate `schema_version == "audit_chain.v1"`.
- **Run report consumers:** treat `replay_metadata` as an additive JSON schema field when parsing `--output json` run summaries.
- **Evidence consumers:** parse versioned `evidence_schema_version` and validate `audit.session_final_hash` linkage to the referenced audit file.
- **Reliability consumers:** parse `reliability_metrics` as additive; provider-internal retries not surfaced by session APIs remain outside current counters.
- **SLO policy consumers:** treat `violations` and `skipped_checks` as machine-readable output (`--output json`) and use `--strict` to fail on missing metrics/insufficient sample.
- **Trend policy consumers:** use `sample_counts` and `skipped` in `akmon slo trend --output json` to separate true regressions from insufficient baseline coverage.
- **Nested automation behavior:** workflows that previously relied on implicit nested write/shell/network approvals must now use explicit parent policy configuration or perform those actions in the primary session.
- **Policy governance rollout:** teams can codify environment profiles and packs without changing permission classes; policy drift is observable via replay/evidence `policy_hash`.

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
