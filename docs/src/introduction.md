# Akmon

<pre style="line-height:1.2;font-family:monospace;color:#f59e0b;text-align:center">
    ✦    ✦  ✦

  ▓▓▓▓▓▓▓▓▓▓▓▓
▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
▓▓▓▓▓▓▓▓▓▓▓▓▓▓
      ▓▓▓▓▓▓▓▓
    ▓▓▓▓▓▓▓▓▓▓▓▓
  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
</pre>

**The AI coding agent built for developers who take security seriously.**

Akmon is a local-first, trust-first AI coding agent written in Rust.
Single binary. No runtime dependencies. Works with any LLM provider.
Logs every action to an audit trail.

**Project site (landing + hosted book):** [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/)

## Why Akmon?

Most AI coding products ask you to trust their stack, billing, and roadmap.
Akmon is the opposite: **you run the binary**, **you pick the model**, and
**you get an explicit permission trail** when you want accountability.

- **Audit-friendly by default.** Tool calls, policy decisions, and session flow
  can be written to JSONL for review later.

- **Sandboxed workspace.** Paths stay under your repo root; optional
  `web_fetch` is guarded against common SSRF patterns; keys are handled
  carefully (zeroized in memory where applicable).

- **No subscription for the agent.** Use Ollama offline or plug in any
  supported cloud provider with your own keys.

- **Single binary.** Roughly 3–4MB without semantic indexing (larger with `--index`). Drop it in
  `/usr/local/bin` and it works over SSH, in Docker, or in CI.

- **Apache 2.0.** Permissive license suited to developer tools and agent integrations ([License](./license.md)).

[Other tools vs Akmon](./comparison.md) — a short, non-billboard contrast when you need it.

## What Akmon does

```bash
# Interactive TUI — describe tasks in plain language
akmon chat

# Plan before implementing — no files touched
akmon --plan --task "refactor the auth module"

# Three-phase spec workflow
akmon spec payment-flow "Stripe integration with webhooks"

# Headless for scripting and CI
akmon --yes --task "fix all clippy warnings"

# Structured output for automation (including early config errors in JSON mode)
akmon --yes --output json --task "list TODO comments" | jq .

# Import context from Claude Code, Cursor, Codex, and others
akmon import

# Export AKMON.md to any other tool format
akmon export --all
```

## Who uses Akmon

- **Security-conscious developers** who need every AI action logged
  for compliance or peace of mind

- **Teams in regulated industries** where code cannot leave the
  machine or must be audited

- **Developers burned by vendor lock-in** who want BYOK with any
  provider including local models

- **Terminal-native developers** who do not want to live in VS Code
  to get good AI assistance

- **Automation authors** composing headless runs, CI jobs, and multi-step “agent” scripts ([Tutorials](./tutorials/overview.md))

## Quick navigation

- New to Akmon? → [Installation](./getting-started/installation.md)
- Already installed? → [Quick Start](./getting-started/quickstart.md)
- **Tutorials (step-by-step, multi-agent, architecture)** → [Tutorials overview](./tutorials/overview.md)
- Full capability map → [Capabilities](./reference/capabilities.md)
- Want examples? → [Language Guides](./languages/rust.md)
- Need a reference? → [CLI Reference](./reference/cli.md)

---

*Named after ἄκμων — the anvil in ancient Greek.
The forge surface where metal is shaped.*
