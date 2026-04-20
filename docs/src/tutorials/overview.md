# Tutorials overview

These tutorials are written for experienced developers who want production-style usage, not toy prompts.

## Before you start

Complete:

- [Installation](../getting-started/installation.md)
- [Quick start](../getting-started/quickstart.md)
- optional provider setup for your preferred model

Recommended baseline command:

```bash
akmon --version
```

## Learning path

| Tutorial | Outcome |
| --- | --- |
| [Step-by-step](./step-by-step.md) | Build real projects in Rust, Python, TypeScript, and refactoring flows |
| [Local-first developer flow (Ollama)](./local-first-ollama.md) | End-to-end local run with evidence + verification |
| [CI headless governance flow](./ci-headless-governance.md) | Run JSON/evidence + enforce SLO/trend gates |
| [Enterprise policy rollout](./enterprise-policy-rollout.md) | Roll `dev` -> `staging` -> `prod` with policy packs |
| [Example projects](./example-projects.md) | Rust, Python, Node starter command recipes |
| [Multi-agent automation](./multi-agent-automation.md) | Use phased workflows and context discipline at scale |
| [Architecture patterns](./architecture-patterns.md) | Select plan/architect/spec patterns by task shape |

## Suggested order by role

### Individual developer

1. `step-by-step`,
2. `architecture-patterns`,
3. `multi-agent-automation`.

### Platform/DevOps engineer

1. `step-by-step` (one stack),
2. `multi-agent-automation`,
3. [headless mode](../usage/headless.md).

### Maintainer handling large refactors

1. `architecture-patterns`,
2. `step-by-step` tutorial 4 (existing codebase refactor),
3. [audit log](../features/audit-log.md).

## Troubleshooting prerequisites

- If provider calls fail, verify keys and model names first.
- If sessions drift, create/update `AKMON.md` before continuing.
- If costs rise unexpectedly, use phased workflow and watch `/context` + `/cost`.

Related: [language guides](../languages/rust.md), [plan mode](../usage/plan-mode.md), [architect mode](../usage/architect-mode.md), [headless mode](../usage/headless.md).
