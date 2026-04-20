# Akmon

Akmon is a terminal-native AI coding agent built as a single Rust binary for teams that need real control over AI side effects. It runs with local or hosted models, enforces typed permission checks for writes/shell/network actions, and produces machine-verifiable artifacts for audit and CI.

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![Passed tests](https://img.shields.io/github/actions/workflow/status/radotsvetkov/akmon/ci.yml?branch=main&label=passed%20tests)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## Why v1.8.0 matters

Akmon v1.8.0 turns trust controls into an operator workflow:

- policy-as-code with deterministic rule evaluation,
- tamper-evident audit chains and verification,
- replay metadata and evidence artifacts for forensic reproducibility,
- reliability metrics with enforceable SLO and trend regression gates,
- enterprise policy profiles/packs (`dev`, `staging`, `prod`) with effective-policy inspection.

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

## What changed in 1.8.0

- Policy-as-Code: docs [Security model](docs/src/features/security.md), [Configuration reference](docs/src/reference/config.md)
- Audit chain + verify command: docs [Audit log](docs/src/features/audit-log.md), [CLI reference](docs/src/reference/cli.md)
- Replay metadata + evidence artifacts: docs [Evidence artifact](docs/src/features/evidence.md)
- Reliability + SLO + trend checks: docs [Reliability & SLO metrics](docs/src/features/reliability-slos.md)
- Enterprise policy profiles/packs: docs [Policy profiles & packs](docs/src/features/policy-profiles.md)

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
