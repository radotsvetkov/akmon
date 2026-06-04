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

Akmon is the review-aware AI coding agent for regulated engineering. Every session is recorded as a tamper-evident, content-addressed, replayable artifact — a deterministic event journal with cryptographic chain integrity, byte-level replay validation, and exportable evidence bundles. Built as a single Rust binary for teams that need real control over AI side effects: typed permission checks for writes, shell, and network; local or hosted model support; machine-verifiable artifacts for audit and CI.

**Website:** [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/) · **Docs:** [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![Passed tests](https://img.shields.io/github/actions/workflow/status/radotsvetkov/akmon/ci.yml?branch=main&label=passed%20tests)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## Why this exists

Most AI coding agents make decisions you cannot audit. Prompt context, model responses, tool calls, and file edits live in process memory and disappear when a session ends. When a regulator, security reviewer, or incident response team asks why an agent made a change, the answer is often somewhere between "we do not know" and "we can try to reproduce it."

Akmon records every prompt, model response, tool call, and file change as a content-addressed event journal with a cryptographic chain. Sessions replay deterministically against recorded providers and tools. Sessions can be compared structurally and at field level. Sessions export as portable evidence bundles in the AGEF format.

Akmon is built for aerospace (DO-178C tool qualification), medical devices (IEC 62304), automotive (ISO 26262), finance (SOC 2 evidence), defense (CMMC), and any environment where code review is a regulatory requirement rather than a cultural preference. If "the AI did it" is not an acceptable explanation, Akmon is for you.

## What's in v2.x

**Latest: [v2.1.0](https://github.com/radotsvetkov/akmon/releases/tag/v2.1.0)** — stability release: session resume, repeat-limit crash fix, tool schema validation, config.toml wiring, git sandbox hardening, and scout/diff dry-run workflows. See [release notes](https://radotsvetkov.github.io/akmon/docs/releases/v2.1.0.html).

**v2.0.0** shipped ten commands organized around the session lifecycle: `run` for normal agent sessions, `replay` for deterministic re-execution against recorded providers and tools, `diff` for structural and field-level session comparison, `inspect` for examining session contents, `bundle` for portable AGEF archives, `redact` for compliance-driven content removal, `audit` for cryptographic chain verification, `evidence` for compliance artifact generation, `verify` for integrity checks, and `doctor` for environment diagnostics. Plus policy profiles, MCP governance, and local model support carried forward from the 1.x line.

## Quickstart

```bash
# Install one binary (example: Linux x86_64)
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 -o ~/bin/akmon
chmod +x ~/bin/akmon

# Verify install
akmon --version

# Run a minimal headless task
cd your-project
akmon --yes --task "summarize failing tests and propose minimal fixes"
```

For verification and audit workflows, use the trust pipeline below.

## Trust pipeline

Akmon's verification pipeline lets you prove a session ran as recorded, with cryptographic chain integrity and SLO compliance, without trusting the agent itself.

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

## Session evidence format (AGEF)

Akmon's session records conform to the [AGEF specification](https://github.com/radotsvetkov/agef), a portable, content-addressed, tamper-evident format for AI agent session evidence. Akmon implements AGEF v0.1.2 and produces bundles intended to be verifiable and portable across environments. Bundles can be optionally signed with an offline Ed25519 key (`akmon bundle sign`); the signature is verifiable by `akmon bundle verify --verify-key` or the standalone `agef-verify --verify-key`, turning a tamper-evident record into a third-party-attributable one.

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

See [CHANGELOG.md](CHANGELOG.md) for release history.

## Documentation

- Project site: [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/)
- Hosted handbook: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)
- Introduction: [docs/src/introduction.md](docs/src/introduction.md)
- Headless mode: [docs/src/usage/headless.md](docs/src/usage/headless.md)
- Interactive mode: [docs/src/usage/interactive.md](docs/src/usage/interactive.md)
- Tutorials: [docs/src/tutorials/overview.md](docs/src/tutorials/overview.md)
- Session diff reference: [docs/src/reference/diff.md](docs/src/reference/diff.md)

## Contributing

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](SECURITY.md)

## License

Apache-2.0 only. See [LICENSE](LICENSE).

---

### What "Akmon" means

Akmon is named after the forge/anvil idea: shape complex code with pressure and precision, while keeping control over every strike (permissions, audit trail, and model choice).
