<div align="center">

<pre>
          ✦         ✦         ✦

    ▓▓▓▓▓
    ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
      ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
        ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                  ▓▓▓▓▓▓▓▓▓▓▓▓
                   ▓▓    ▓▓
                   ▓▓    ▓▓
               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
            ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
         ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
      ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
</pre>

# Akmon

**The AI coding agent built for developers
who take security seriously.**

`local-first` · `trust-first` · `rust-native` ·
`single binary` · `no subscription`

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-286%2B_passing-brightgreen.svg)](https://github.com/radotsvetkov/akmon/actions)

</div>

---

## Why Akmon

Most hosted coding agents ask you to hand over trust by default: your 
repo, your keys, and your workflow ride inside someone else’s product, 
pricing, and uptime. We think you should **see** what happened, **own** 
the binary, and **choose** who gets paid—not rent the forge.

Akmon is different by design:

| | Akmon | Typical agents |
| --- | :---: | :---: |
| **Full JSONL audit trail** (policy, tools, outcomes) | ✅ | ❌ |
| **Explicit policy engine** + confirm/deny path | ✅ | ⚠️ varies |
| **Sandboxed to your git root** (path hardening) | ✅ | ⚠️ varies |
| **SSRF-hardened** optional web fetch | ✅ | ⚠️ varies |
| **Secrets as `Secret<T>`** (zeroized, not logged) | ✅ | ❌ |
| **Single static binary** — no extra runtime | ✅ | ❌ |
| **Runs offline** (local model runtime) | ✅ | ❌ |
| **Bring your own keys / endpoints** | ✅ | ⚠️ often tied to vendor |
| **Open source** — build, fork, pin your version | ✅ | ❌ |
| **No product subscription** for the agent itself | ✅ | ❌ |

**Every action is audited.** Every tool call, permission decision, and 
model turn can land in a timestamped JSONL file—so we can answer 
“what ran, why was it allowed, and what changed?” long after the session.

**Nothing leaves your machine without your rules.** Paths are resolved 
against the repo sandbox; network fetch is off until you opt in and 
still blocks common abuse targets. The policy layer is part of the 
product, not an afterthought.

**You keep the keys and the binary.** Point Akmon at the APIs or 
compatible endpoints you already pay for, use a model router with one 
key, plug in cloud inference, or stay entirely on a local stack—one 
forge you can ship, script, and run in CI, under your control.

---

## Install

### One-line install (macOS/Linux)

```bash
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m) \
  -o /usr/local/bin/akmon && chmod +x /usr/local/bin/akmon
```

### From source

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon
cargo build --release
cp target/release/akmon /usr/local/bin/
```

Requires Rust 1.88+ via [rustup](https://rustup.rs).

---

## Quick start

### Local models (free, offline, private)

```bash
# Install Ollama: https://ollama.com
ollama pull qwen2.5-coder:7b

akmon chat
```

### Cloud API (frontier model)

```bash
export ANTHROPIC_API_KEY=your-key

akmon chat --model claude-haiku-4-5-20251001
```

### OpenRouter (500+ models, one key)

```bash
export OPENROUTER_API_KEY=your-key

# Use any model through one key
akmon chat --model anthropic/claude-haiku-4-5
akmon chat --model meta-llama/llama-3.3-70b-instruct
akmon chat --model deepseek/deepseek-chat
```

### Set up once with the config wizard

```bash
akmon config
```

---

## Interactive TUI

<table>
<tr>
<td>
akmon v1.4.0  │  project: my-app  │  your-model  │  INTERACTIVE
──────────────────────────────────────────────────────────────────
You: find the auth code and explain how tokens work
→ semantic_search
✓ semantic_search                            [Tab to expand]
→ read_file
✓ read_file
Akmon: The authentication system uses JWT tokens stored
in Redis with a 24-hour TTL. Refresh tokens are persisted
to PostgreSQL. The middleware in src/auth/jwt.rs validates
incoming requests using the HS256 algorithm...  ▊
──────────────────────────────────────────────────────────────────
a1b2c3d4  │  tokens:4821  │  cache:8779  │  step 2/25
──────────────────────────────────────────────────────────────────

type a message or / for commands


</td>
</tr>
</table>

The `cache:8779` means 8,779 tokens were served from the model
host’s prompt cache—often far cheaper than fresh tokens for repeated
context. We surface that number so you can see what the session saved.

---

## Headless mode (CI and scripting)

```bash
# Run a task
akmon --yes --task "add error handling to the fetch function"

# JSON output for scripting
akmon --yes --output json --task "list all TODO comments" | jq .result

# Plan before implementing
akmon --plan --task "refactor the auth module"
cat .akmon/plans/*.md  # review the plan

# Architect mode: cheap model plans, main model implements
akmon --architect \
  --planner-model llama3.2 \
  --model claude-haiku-4-5-20251001 \
  --task "add OAuth to the API"
```

---

## Spec-driven development

For building new features from scratch with structured planning:

```bash
# Generate requirements → design → tasks
akmon spec auth-system \
  "JWT authentication with refresh tokens"

# Review and iterate on each phase
akmon spec auth-system design
akmon spec auth-system tasks

# Implement one task at a time
akmon spec auth-system implement
```

---

## Project initialization

```bash
# Analyze an existing project and generate AKMON.md
cd my-existing-project
akmon init

# Scaffold a new project from scratch
akmon new my-api --lang rust --type cli \
  "A CLI tool for processing CSV files"
```

---

## The audit trail

Every session writes a JSONL audit log:

```bash
cat .akmon/audit/$(ls .akmon/audit | tail -1) | jq .
```

```json
{"event_kind":"policy_evaluation",
 "permission":{"permission":"write_file","path":"src/main.rs"},
 "verdict":"allow","reason":"user confirmed"}
{"event_kind":"agent_step",
 "description":"ToolCallCompleted(edit, success=true)"}
```

Every policy decision. Every tool call. Every permission grant or denial. 
Logged, timestamped, machine-readable. This is what compliance teams need 
and what no other tool provides.

---

## Provider support

| Provider | How |
| --- | --- |
| Ollama (local) | `akmon chat --model llama3.2` |
| Anthropic | `ANTHROPIC_API_KEY=... akmon chat` |
| OpenRouter | `OPENROUTER_API_KEY=... akmon chat --model anthropic/claude-haiku` |
| OpenAI | `OPENAI_API_KEY=... akmon chat --model gpt-4o` |
| Groq | `GROQ_API_KEY=... akmon chat --model llama-3.3-70b-versatile` |
| Azure OpenAI | `akmon chat --azure-endpoint ... --azure-key ...` |
| Amazon Bedrock | `AWS_... akmon chat --bedrock` |
| Any OpenAI-compatible | `akmon chat --openai-compatible-url ...` |

---

## Tools

| Tool | What it does | Permission |
| --- | --- | --- |
| `list_directory` | Explore project structure | read (--yes) |
| `read_file` | Read any text file | read (--yes) |
| `write_file` | Atomic file write | confirm always |
| `edit` | Surgical string replace | confirm always |
| `patch` | Apply unified diff | confirm always |
| `search` | Regex search with context | read (--yes) |
| `semantic_search` | Natural language code search | read (--yes) |
| `git` | Status, diff, log, add, commit | read/confirm |
| `shell` | Allowlisted commands only | confirm always |
| `web_fetch` | SSRF-protected URL fetch | opt-in |
| MCP tools | Any MCP server tools | confirm |

---

## Security model

**What `--yes` approves:** ReadFile, ListDirectory, SemanticSearch, 
Search, GitStatus, GitDiff, GitLog — read-only operations only.

**What requires confirmation regardless:** WriteFile, EditFile, 
PatchFile, GitAdd, GitCommit, ShellTool, WebFetch.

**What is structurally prevented:**
- Path traversal: all paths canonicalized against git root before any operation
- SSRF: web fetch blocks RFC1918, loopback, cloud metadata endpoints
- Prompt injection: file contents always isolated in structural delimiters
- Credential leakage: API keys stored as `Secret<T>`, zeroized on drop, 
  never appear in logs or debug output

---

## Configuration

```bash
akmon config                    # interactive wizard
akmon config show               # print current config
akmon config model list         # list available models
akmon config model set llama3.2 # set default model
akmon config mcp add github \   # add MCP server
  https://mcp.github.com
akmon config key set anthropic  # store API key
```

Config lives at `~/.akmon/config.toml` — TOML format, 
supports comments, no trailing comma issues.

---

## AKMON.md — project memory

Create `AKMON.md` at your project root or generate it:

```bash
akmon init  # analyzes project and generates AKMON.md
```

Akmon reads this at session start. Structure it as:

```markdown
# My Project

## Product
What this is and who it is for.

## Architecture  
Key components and how they relate.

## Conventions
Error handling, naming, testing patterns the AI must follow.

## Current sprint
What you are building THIS WEEK.
Update this before each session.
```

The `## Current sprint` section is the most important. 
It tells Akmon what you are working on and dramatically 
reduces context drift across sessions.

---

## Project structure

| Crate | Responsibility |
| --- | --- |
| `akmon-cli` | Binary entry point, CLI args, subcommands |
| `akmon-core` | Sandbox, policy engine, FSM, audit log, secrets |
| `akmon-config` | Config file (`~/.akmon/config.toml`), provider detection |
| `akmon-models` | LLM backends — Ollama, Anthropic, OpenAI-compatible, Bedrock |
| `akmon-tools` | Tool implementations — file, git, shell, web fetch, MCP |
| `akmon-query` | Agent session, context management, summarization |
| `akmon-index` | Semantic indexing with fastembed (optional feature) |
| `akmon-tui` | ratatui TUI, slash commands, session UI |

Documentation in `docs/`:
[architecture](docs/architecture.md) ·
[security](docs/security.md) ·
[data flows](docs/data-flows.md)

Project memory: [`AKMON.md`](AKMON.md)

---

## Building from source

```bash
# Standard build
cargo build --release

# Without semantic indexing (smaller binary)
cargo build --release --no-default-features

# Run tests
cargo test --workspace
```

---

## License

Licensed under [MIT](LICENSE-MIT) or 
[Apache 2.0](LICENSE-APACHE) at your option — 
the standard Rust ecosystem dual license.

---

<div align="center">

---

Built with [ratatui](https://ratatui.rs) ·
Powered by [fastembed](https://github.com/Anush008/fastembed-rs)

*Named after* **ἄκμων** *— the anvil in ancient Greek.*
*The forge surface where metal is shaped.*
*Where code is hammered into form.*

</div>
