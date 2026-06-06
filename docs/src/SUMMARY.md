# Summary

[Introduction](./introduction.md)

# Concepts and trust

- [Trust model](./concepts/trust-model.md)
- [Architecture](./concepts/architecture.md)
- [Verifying evidence](./concepts/verifying-evidence.md)
- [Compliance](./concepts/compliance.md)
- [Glossary](./concepts/glossary.md)
- [Regulated reviewer flow](./concepts/reviewer-flow.md)
- [Other tools vs Akmon](./comparison.md)

---

# Evidence and verification

- [akmon bundle keygen](./reference/bundle-keygen.md)
- [akmon sign](./reference/sign.md)
- [akmon bundle attest](./reference/bundle-attest.md)
- [akmon bundle verify](./reference/bundle-verify.md)
- [agef-verify](./reference/agef-verify.md)
- [akmon bundle prove-openssl](./reference/bundle-prove-openssl.md)
- [akmon bundle export](./reference/bundle-export.md)
- [akmon bundle import](./reference/bundle-import.md)
- [akmon inspect](./reference/inspect.md)
- [akmon redact](./reference/redact.md)
- [akmon replay](./reference/replay.md)
- [akmon diff](./reference/diff.md)
- [akmon verify](./reference/verify.md)

---

# Use cases

- [Record who approved an AI change](./use-cases/operator-sign-off.md)
- [Verify evidence on an air-gapped machine](./use-cases/air-gapped-audit.md)
- [Assemble a signed evidence pack for a release](./use-cases/release-evidence-pack.md)

---

# Tutorials

- [Tutorials overview](./tutorials/overview.md)
- [Local-first developer flow (Ollama)](./tutorials/local-first-ollama.md)
- [CI headless governance flow](./tutorials/ci-headless-governance.md)
- [Third-party OTEL trace to offline openssl proof](./tutorials/otel-to-openssl-walkthrough.md)
- [Enterprise policy rollout](./tutorials/enterprise-policy-rollout.md)

---

# Getting Started

- [Installation](./getting-started/installation.md)
- [Quick Start](./getting-started/quickstart.md)
- [Provider Setup](./getting-started/providers.md)
- [Configuration](./getting-started/configuration.md)

---

# Providers

- [Ollama (Local)](./providers/ollama.md)
- [Anthropic](./providers/anthropic.md)
- [OpenRouter](./providers/openrouter.md)
- [OpenAI](./providers/openai.md)
- [Groq](./providers/groq.md)
- [Azure OpenAI](./providers/azure.md)
- [Amazon Bedrock](./providers/bedrock.md)
- [Custom Endpoints](./providers/custom.md)

---

# Bundled reference agent

The bundled coding agent is the reference, gold-fidelity producer. Use it when you want a full recording that replays deterministically. The evidence and verification layer above works with any producer through OpenTelemetry.

- [Interactive Mode](./usage/interactive.md)
- [Headless Mode](./usage/headless.md)
- [Plan Mode](./usage/plan-mode.md)
- [Architect Mode](./usage/architect-mode.md)
- [Spec Workflow](./usage/spec-workflow.md)

## Project setup

- [akmon init](./project/init.md)
- [AKMON.md Reference](./project/akmon-md.md)
- [Importing Context](./project/import.md)
- [Exporting Context](./project/export.md)

## Language guides

- [Rust Projects](./languages/rust.md)
- [Python Projects](./languages/python.md)
- [TypeScript Projects](./languages/typescript.md)
- [Go Projects](./languages/go.md)
- [Other Languages](./languages/other.md)

## Agent features

- [Semantic Search](./features/semantic-search.md)
- [Git Integration](./features/git.md)
- [MCP Tools](./features/mcp.md)
- [Audit Log](./features/audit-log.md)
- [Policy Profiles and Packs](./features/policy-profiles.md)
- [Evidence Artifact](./features/evidence.md)
- [Security Model](./features/security.md)
- [Reliability and SLO Metrics](./features/reliability-slos.md)
- [Cost Transparency](./features/cost.md)

---

# Reference

- [Capabilities](./reference/capabilities.md)
- [CLI Reference](./reference/cli.md)
- [Slash Commands](./reference/slash-commands.md)
- [Configuration Reference](./reference/config.md)
- [Tools Reference](./reference/tools.md)
- [Environment Variables](./reference/env-vars.md)
- [Release notes: v2.2.0](./releases/v2.2.0.md)
- [Release notes: v2.1.0](./releases/v2.1.0.md)
- [Release notes: v2.0.0](./releases/v2.0.0.md)
- [Release notes: v1.8.2](./releases/v1.8.2.md)
- [Release notes: v1.8.1](./releases/v1.8.1.md)
- [Release notes: v1.8.0](./releases/v1.8.0.md)

---

# Contributing

- [Development Setup](./contributing/setup.md)
- [Architecture](./contributing/architecture.md)
- [Adding a Provider](./contributing/providers.md)
- [Changelog](./contributing/changelog.md)

---

[Security Policy](./security.md)
[License](./license.md)
