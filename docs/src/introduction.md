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

## Why Akmon?

Most AI coding tools require you to trust a vendor with your code,
your API keys, and your workflow. Anthropic can block your access
overnight. Cursor can change billing without warning. Claude Code
requires a subscription you do not control.

Akmon is different by design:

- **Every action is audited.** Every tool call, permission decision,
  and model response is logged to a JSONL file. No other terminal
  coding agent does this.

- **Nothing leaves without permission.** Files are sandboxed to your
  git root. Web requests are SSRF-protected. API keys are zeroized
  in memory on drop.

- **No subscription. No lock-in.** Bring your own API key for any
  provider — or run fully offline with Ollama. You own the tool.

- **Single binary.** Roughly 3–4MB without semantic indexing (larger with `--index`). Drop it in
  `/usr/local/bin` and it works. Works over SSH. Works in Docker.
  Works in CI.

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

# Import context from Claude Code, Cursor, Kiro, and others
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

## Quick navigation

- New to Akmon? → [Installation](./getting-started/installation.md)
- Already installed? → [Quick Start](./getting-started/quickstart.md)
- Want examples? → [Language Guides](./languages/rust.md)
- Need a reference? → [CLI Reference](./reference/cli.md)

---

*Named after ἄκμων — the anvil in ancient Greek.
The forge surface where metal is shaped.*
