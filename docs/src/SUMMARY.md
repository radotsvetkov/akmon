# Summary

[Introduction](./introduction.md)

# Start here

- [Quick start](./getting-started/quickstart.md)
- [Install](./getting-started/installation.md)

# Concepts

- [Trust model](./concepts/trust-model.md)
- [Capture levels](./concepts/capture-levels.md)
- [What verification proves](./concepts/verifying-evidence.md)
- [How Akmon works](./concepts/architecture.md)
- [Glossary](./concepts/glossary.md)
- [Akmon and other tools](./comparison.md)

# Guides

- [Seal an OpenTelemetry trace from any agent](./guides/seal-otel-trace.md)
  - [Walkthrough: trace to OpenSSL proof](./tutorials/otel-to-openssl-walkthrough.md)
- [Hand evidence to an auditor](./guides/hand-to-auditor.md)
- [Verify on an air-gapped machine](./use-cases/air-gapped-audit.md)
- [Record who approved a session](./use-cases/operator-sign-off.md)
- [Assemble a release evidence pack](./use-cases/release-evidence-pack.md)
- [Regulated reviewer flow](./concepts/reviewer-flow.md)
- [Trusted timestamps](./guides/trusted-timestamps.md)
- [Retention and archiving](./guides/retention.md)

# Compliance

- [Compliance crosswalk](./concepts/compliance.md)

# Reference

- [akmon seal](./reference/seal.md)
- [akmon otel import](./reference/otel-import.md)
- [akmon bundle keygen](./reference/bundle-keygen.md)
- [akmon bundle sign](./reference/bundle-sign.md)
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
- [CLI reference](./reference/cli.md)
- [Capabilities](./reference/capabilities.md)
- [Configuration reference](./reference/config.md)
- [Environment variables](./reference/env-vars.md)

# Bundled reference agent

- [Interactive mode](./usage/interactive.md)
  - [Headless mode](./usage/headless.md)
  - [Plan mode](./usage/plan-mode.md)
  - [Architect mode](./usage/architect-mode.md)
  - [Provider setup](./getting-started/providers.md)
    - [Ollama (local)](./providers/ollama.md)
    - [Anthropic](./providers/anthropic.md)
    - [OpenRouter](./providers/openrouter.md)
    - [OpenAI](./providers/openai.md)
    - [Groq](./providers/groq.md)
    - [Azure OpenAI](./providers/azure.md)
    - [Amazon Bedrock](./providers/bedrock.md)
    - [Custom endpoints](./providers/custom.md)
  - [Configuration](./getting-started/configuration.md)
  - [akmon init](./project/init.md)
  - [AKMON.md reference](./project/akmon-md.md)
  - [Policy profiles and packs](./features/policy-profiles.md)
  - [Security model](./features/security.md)
  - [MCP tools](./features/mcp.md)
  - [Git integration](./features/git.md)
  - [Cost transparency](./features/cost.md)
  - [Audit log (legacy)](./features/audit-log.md)
  - [Evidence artifact (legacy)](./features/evidence.md)
  - [Tools reference](./reference/tools.md)
  - [Slash commands](./reference/slash-commands.md)
  - [Rust projects](./languages/rust.md)
  - [Python projects](./languages/python.md)
  - [TypeScript projects](./languages/typescript.md)
  - [Go projects](./languages/go.md)
  - [Other languages](./languages/other.md)
  - [Tutorials](./tutorials/overview.md)
    - [Local-first flow (Ollama)](./tutorials/local-first-ollama.md)
    - [CI headless governance](./tutorials/ci-headless-governance.md)
    - [Enterprise policy rollout](./tutorials/enterprise-policy-rollout.md)

# Releases

- [v3.0.0](./releases/v3.0.0.md)
- [v2.2.1](./releases/v2.2.1.md)
- [v2.2.0](./releases/v2.2.0.md)
- [v2.1.0](./releases/v2.1.0.md)
- [v2.0.0](./releases/v2.0.0.md)
- [v1.8.2](./releases/v1.8.2.md)
- [v1.8.1](./releases/v1.8.1.md)
- [v1.8.0](./releases/v1.8.0.md)

# Contributing

- [Development setup](./contributing/setup.md)
- [Architecture](./contributing/architecture.md)
- [Adding a provider](./contributing/providers.md)
- [Changelog](./contributing/changelog.md)

---

[Security policy](./security.md)
[License](./license.md)
