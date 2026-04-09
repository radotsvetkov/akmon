# Capabilities reference

This page is a practical map of what Akmon can do and how to choose the right mode.

## Runtime and packaging

| Capability | Why it matters |
| --- | --- |
| Single Rust binary | predictable behavior across laptop, SSH host, CI runner |
| Optional feature set | choose slim or full builds by environment needs |
| Terminal-first UX | works where editor plugins are unavailable |

## Operating modes

| Mode | Command | Best use case |
| --- | --- | --- |
| Interactive | `akmon chat` | supervised iterative implementation |
| Headless | `akmon --yes --task "..."` | CI and automation |
| JSON reporting | `--output json` | machine-readable orchestration |
| Plan-only | `--plan` | read-only scoping before edits |
| Architect | `--architect` | plan+implement with model split |
| Spec workflow | `akmon spec ...` | structured requirements/design/tasks |

## Model/provider support

Akmon supports local and cloud providers, including:

- Ollama (offline/local),
- Anthropic,
- OpenAI-compatible providers,
- OpenRouter, Groq, Azure, Bedrock.

Model selection is per-task, enabling cost/capability optimization.

## Core tooling capabilities

- file ops (read/write/edit/patch),
- search (text and optional semantic),
- git context and git actions,
- shell commands (policy constrained),
- network fetch with protections,
- MCP integrations for external systems.

## Policy and safety capabilities

- permission-gated side effects,
- write diff confirmation flows,
- sandboxed filesystem boundaries,
- auditable tool + policy events.

## Context and memory capabilities

- `AKMON.md` project steering,
- `.akmon/specs` persistent plan artifacts,
- session continuation (`-c`) with resumable context,
- todo and memory primitives for multi-turn continuity.

## Cost and observability capabilities

- token and cache visibility in UI,
- cost estimates and run summaries,
- JSONL audit trail for runtime evidence.

## Automation capabilities

- headless runs with budget caps,
- structured JSON run output,
- script-friendly command model for batch operations.

## Known non-goals

- no hosted SaaS runtime (you run it),
- no mandatory IDE dependency,
- no guarantee that third-party model APIs are available.

Next steps: [tutorials overview](../tutorials/overview.md), [headless mode](../usage/headless.md), [security model](../features/security.md).
