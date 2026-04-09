# Capabilities reference

This page is a **map** of what Akmon can do; deep dives live in linked chapters.

## Runtime & packaging

| Capability | Notes |
| --- | --- |
| Single static binary | No Node/Python runtime required to run the CLI |
| Optional semantic index | Smaller build with `--no-default-features` ([Semantic search](../features/semantic-search.md)) |
| SSH / Docker / CI | Same binary; configure via env and flags |

## Modes of operation

| Mode | Entry | Best for |
| --- | --- | --- |
| Interactive TUI | `akmon chat` | Exploration, diffs before writes, slash commands ([Interactive](../usage/interactive.md)) |
| Headless | `akmon --yes --task "…"` | Scripts, batch refactors ([Headless](../usage/headless.md)) |
| JSON output | `--output json` | Pipelines; structured early errors when config is invalid |
| Plan | `--plan` | Analysis and written plans only ([Plan mode](../usage/plan-mode.md)) |
| Architect | `--architect`, `--planner-model` | Planner/implementer split ([Architect](../usage/architect-mode.md)) |
| Spec workflow | `akmon spec …` | Requirements → design → tasks ([Spec](../usage/spec-workflow.md)) |

## Providers (BYOK)

Ollama (local), Anthropic, OpenRouter, OpenAI, Groq, Azure OpenAI, Amazon Bedrock, and OpenAI-compatible endpoints. Configuration and env vars: [Provider setup](../getting-started/providers.md) and [Providers](../providers/ollama.md) chapters.

## Tools (built-in)

File read/write, `edit`, unified `patch` / `apply_patch`, regex `search`, optional `semantic_search`, git (status/diff/log and mutating commands where allowed), allowlisted `shell`, SSRF-aware `web_fetch`, plus [MCP](../features/mcp.md) tools when configured.

Permissions default to **confirm writes** even with `--yes` (reads can be auto-approved—see [Security](../features/security.md)).

## Project intelligence

Language and framework hints are detected and injected into context (Rust, Go, Python, TypeScript, Elixir, and more). You steer with `AKMON.md` ([Project setup](../project/init.md)).

## Audit & cost visibility

JSONL audit logs, session summaries, token and cache display in the TUI, estimated USD—see [Audit log](../features/audit-log.md) and [Cost](../features/cost.md).

## Interop

`akmon import` / `akmon export` sync steering files with other AI tools ([Import](../project/import.md), [Export](../project/export.md)).

## When something is *not* built in

- **No hosted multi-tenant service** — you run the binary.
- **No automatic code review** — you review diffs and plans.
- **No guarantee** that cloud APIs stay available — that is between you and the provider.

For tutorials that combine these pieces, start with [Tutorials overview](../tutorials/overview.md).
