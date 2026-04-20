<div align="center">

<pre>
            ✦        ✦        ✦

           ▓▓▓
           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
         ▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒
         ▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒
           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                   ▓▓▓▓▓▓▓▓▓▓▓▓
                    ▓▓      ▓▓
                    ▓▓      ▓▓
                 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
</pre>

</div>

# Akmon

Akmon is a terminal-native AI coding agent built as a single Rust binary for teams that need real control over AI side effects. It runs with local or hosted models, enforces typed permission checks for writes/shell/network actions, and produces machine-verifiable artifacts for audit and CI.

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![Passed tests](https://img.shields.io/github/actions/workflow/status/radotsvetkov/akmon/ci.yml?branch=main&label=passed%20tests)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## Why v1.8.1 matters

Akmon v1.8.1 is an operability + reliability hardening release:

- deterministic provider diagnostics (`akmon doctor providers`),
- fail-closed MCP governance with enriched audit context,
- deterministic docs quality gates in CI,
- internal TUI state decomposition with behavior parity (no UX change),
- stronger Ollama/local timeout and remediation behavior.

## 5-minute quickstart

```bash
# Install one binary (example: Linux x86_64)
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 -o ~/bin/akmon
chmod +x ~/bin/akmon

# Verify
akmon --version

# Optional one-time setup wizard
akmon config

# Run a headless task with structured output
cd your-project
akmon --yes --output json --task "run tests and summarize failures" | tee run.json
```

## Trust & verification pipeline

```bash
# 1) Run a task (writes audit + evidence by default)
akmon --yes --output json --task "apply rustfmt fixes only" | tee run.json

# 2) Verify tamper-evident audit chain
akmon audit verify .akmon/audit/<session-id>.jsonl

# 3) Verify evidence integrity and linked audit hash
akmon evidence verify .akmon/evidence/<session-id>.json

# 4) Enforce single-run SLO policy
akmon slo verify .akmon/evidence/<session-id>.json --strict

# 5) Detect regressions against historical baseline
akmon slo trend .akmon/evidence/<session-id>.json --baseline-dir .akmon/evidence/history --window 20 --strict
```

## Enterprise policy profiles

```bash
# Inspect merged effective policy for production
akmon policy show-effective --profile prod --output json

# Add organizational policy packs
akmon --policy-profile staging \
  --policy-pack .akmon/policy-packs/org.toml \
  --policy-pack .akmon/policy-packs/team.toml \
  --task "run verification commands and report findings"
```

Merge precedence is deterministic:

`profile < packs < project-local policy < CLI override`

## What changed in 1.8.1

- Provider diagnostics command: docs [CLI reference](docs/src/reference/cli.md), [Provider setup](docs/src/getting-started/providers.md)
- MCP governance hardening: docs [Security model](docs/src/features/security.md), [MCP guide](docs/src/features/mcp.md), [Configuration reference](docs/src/reference/config.md)
- Docs quality gates: docs [Contributing guide](CONTRIBUTING.md), [docs/README](docs/README.md)
- TUI internal refactor (no UX change): docs [Contributing architecture](docs/src/contributing/architecture.md)
- Local model reliability improvements: docs [Configuration](docs/src/getting-started/configuration.md), [Cost guide](docs/src/features/cost.md)

## Documentation

- Hosted docs: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)
- Introduction: [docs/src/introduction.md](docs/src/introduction.md)
- Headless mode: [docs/src/usage/headless.md](docs/src/usage/headless.md)
- Interactive mode: [docs/src/usage/interactive.md](docs/src/usage/interactive.md)
- Tutorials: [docs/src/tutorials/overview.md](docs/src/tutorials/overview.md)

## Contributing

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](SECURITY.md)

## License

Apache-2.0 only. See [LICENSE](LICENSE).

---

### What "Akmon" means

Akmon is named after the forge/anvil idea: shape complex code with pressure and precision, while keeping control over every strike (permissions, audit trail, and model choice).
