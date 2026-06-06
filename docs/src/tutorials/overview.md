# Tutorials overview

Documented for Akmon `2.2.0`.

These tutorials are written for experienced developers and reviewers who want production-style usage, not toy prompts. Akmon is a producer-agnostic evidence and verification layer; the tutorials are ordered so the trust chain comes first and the bundled reference agent comes after.

## Before you start

Complete:

- [Installation](../getting-started/installation.md)
- [Quick start](../getting-started/quickstart.md), which walks the full trust flow: keygen, otel import, sign, verify, prove-openssl, openssl
- optional provider setup if you plan to run the bundled reference agent

Recommended baseline command:

```bash
akmon --version
```

## Start here: the trust chain

The single most important walkthrough takes a session from a third-party agent's trace all the way to an offline `openssl` proof a stranger can verify.

| Tutorial | Outcome |
| --- | --- |
| [Third-party OTEL trace to offline openssl proof](./otel-to-openssl-walkthrough.md) | Import an OpenTelemetry trace, sign it, verify it, and prove the signature with plain `openssl`, no Akmon install on the verifier's side |

This is the producer-agnostic path. It does not require Akmon's own agent at all.

## The bundled reference agent

The reference agent is the gold-fidelity producer of full-capture sessions. These tutorials cover producing and governing those sessions.

| Tutorial | Outcome |
| --- | --- |
| [Local-first developer flow (Ollama)](./local-first-ollama.md) | End-to-end local run with evidence and verification |
| [CI headless governance flow](./ci-headless-governance.md) | Run JSON and evidence, enforce SLO and trend gates |
| [Enterprise policy rollout](./enterprise-policy-rollout.md) | Roll `dev`, then `staging`, then `prod` with policy packs |
| [Step-by-step (Rust, Go, Python, Elixir)](./step-by-step.md) | Build real projects across languages and refactoring flows |
| [Example projects](./example-projects.md) | Rust, Python, Node starter command recipes |
| [Multi-agent and automation](./multi-agent-automation.md) | Phased workflows and context discipline at scale |
| [Architecture patterns](./architecture-patterns.md) | Select plan, architect, or spec patterns by task shape |

## Suggested order by role

### Compliance engineer or auditor

1. [OTEL trace to openssl proof](./otel-to-openssl-walkthrough.md),
2. [reviewer flow](../concepts/reviewer-flow.md),
3. [CI headless governance flow](./ci-headless-governance.md).

### Individual developer

1. [OTEL trace to openssl proof](./otel-to-openssl-walkthrough.md),
2. `step-by-step`,
3. `architecture-patterns`.

### Platform or DevOps engineer

1. [OTEL trace to openssl proof](./otel-to-openssl-walkthrough.md),
2. [CI headless governance flow](./ci-headless-governance.md),
3. [headless mode](../usage/headless.md).

### Maintainer handling large refactors

1. `architecture-patterns`,
2. `step-by-step` (existing-codebase refactor),
3. [audit log](../features/audit-log.md).

## Troubleshooting prerequisites

- If `openssl` cannot verify a proof on macOS, you are on LibreSSL. Use OpenSSL 3.x.
- If `akmon bundle sign` rejects a key, regenerate it with `akmon bundle keygen`.
- If `--require-capture full` fails on an imported session, that is expected. Imports are `structural`.
- If provider calls fail in the reference agent, verify keys and model names first.
- If costs rise unexpectedly, use a phased workflow and watch `/context` and `/cost`.

Related: [glossary](../concepts/glossary.md), [reviewer flow](../concepts/reviewer-flow.md), [plan mode](../usage/plan-mode.md), [architect mode](../usage/architect-mode.md), [headless mode](../usage/headless.md).
