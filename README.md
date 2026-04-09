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

A terminal-native AI coding agent in a single Rust binary.

Bring your own model and key (Anthropic, OpenAI, OpenRouter, Groq, Azure, Bedrock), or run fully offline with Ollama. Every sensitive action is permissioned and sessions can be audited as JSONL.

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![Passed tests](https://img.shields.io/github/actions/workflow/status/radotsvetkov/akmon/ci.yml?branch=main&label=passed%20tests)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## Why it exists

Most AI coding tools are optimized for one provider, one IDE, and one trust model. Akmon exists for teams that need:

- provider choice (or fully offline local models),
- terminal portability (SSH, Docker, CI),
- explicit permission boundaries for writes and shell,
- auditability when AI changes production code.

## See it working

- Landing + docs: [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/)
- Tutorials with real workflows: [docs/tutorials](https://radotsvetkov.github.io/akmon/docs/tutorials/overview.html)

Example live session:

```text
You: build a FastAPI service with users CRUD and PostgreSQL
→ Akmon explores project files and usually proposes an implementation plan
→ requests permission before file writes and other sensitive actions
→ implements models/routes/tests incrementally with visible tool steps
→ runs verification commands (for example pytest) when your task and permissions allow it
→ exits with token usage, cache stats, estimated cost, and audit log path
```

## Install now

```bash
# macOS / Linux quick install
curl -L "https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)" \
  -o /usr/local/bin/akmon
chmod +x /usr/local/bin/akmon

# verify
akmon --version
```

Or from source:

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon
cargo build --release
cp target/release/akmon /usr/local/bin/
```

## First session

```bash
cd your-project
export ANTHROPIC_API_KEY=sk-ant-...
akmon chat
```

Offline mode:

```bash
ollama pull qwen3.5:9b
akmon chat --model qwen3.5:9b
```

## What makes Akmon different

| Tool | Strength | Tradeoff |
| --- | --- | --- |
| Akmon | Portable terminal binary, provider independence, permission + audit model, strong headless automation | Less IDE-native UX than editor-integrated tools |
| Claude Code | Excellent Anthropic-first in-terminal experience | Provider coupling to Anthropic |
| Cursor | Deep editor integration and inline flow | IDE dependency; different trust/deployment model |
| Aider | Lightweight terminal workflow, git-centric editing | Different policy/audit and orchestration model |

Short version: Akmon prioritizes portability + accountability over IDE polish.

## Core workflows

```bash
# Interactive TUI
akmon chat

# Headless automation
akmon --yes --output json --task "run cargo clippy and fix warnings"

# Plan-only (no writes)
akmon --plan --task "design a migration from sqlite to postgres"

# Architect mode (planner model + implementer model)
akmon --architect --planner-model llama3.2 --task "add OAuth2 login"
```

## Cost transparency

Akmon surfaces cumulative token usage, cache hits, and cost estimates.

In the interactive TUI, run **`/config`** or press **Ctrl+S** to open **settings** and, on the **Estimates** tab, tune **context window** and optional **USD-per-million** overrides for the active model (stored under **`[[model_estimates]]`** in `~/.akmon/config.toml`). Numbers are **rough estimates**, not a bill; the context **%** bar measures prompt fill versus window size, not provider RPM/TPM limits.

Real session example:

- input: 672k tokens @ $0.80/M
- output: 35k tokens @ $4.00/M
- cache reads: 258k tokens @ $0.08/M
- total: about $0.68

## Security model

- Read operations can be auto-approved with `--yes`.
- Writes, shell commands, and network fetches remain policy-checked.
- Paths are sandboxed to project roots.
- Optional JSONL audit logs record what happened and when.

## Documentation

- Hosted docs: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)
- Introduction: [docs/src/introduction.md](docs/src/introduction.md)
- Headless mode: [docs/src/usage/headless.md](docs/src/usage/headless.md)
- MCP guide: [docs/src/features/mcp.md](docs/src/features/mcp.md)
- Multi-agent automation: [docs/src/tutorials/multi-agent-automation.md](docs/src/tutorials/multi-agent-automation.md)

## Contributing

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](SECURITY.md)
- Developer docs: [docs/src/contributing/setup.md](docs/src/contributing/setup.md)

Community practices align with [Open Source Guides](https://opensource.guide/) (documentation, security reporting, and welcoming contributions).

## License

Apache-2.0 only. See [LICENSE](LICENSE).

---

### What "Akmon" means

Akmon is named after the forge/anvil idea: shape complex code with pressure and precision, while keeping control over every strike (permissions, audit trail, and model choice).
