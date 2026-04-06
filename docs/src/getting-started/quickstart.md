# Quick Start

From zero to a working AI coding session in under 5 minutes.

## Step 1 — Choose a model provider

### Local (free, private, offline)

Install [Ollama](https://ollama.com) and pull a model:

```bash
ollama pull qwen2.5-coder:7b
```

Akmon detects Ollama automatically. No key needed.

### Cloud API

```bash
# Anthropic — best quality
export ANTHROPIC_API_KEY=your-key

# OpenRouter — 500+ models with one key
export OPENROUTER_API_KEY=your-key

# Groq — fastest inference
export GROQ_API_KEY=your-key
```

## Step 2 — Open your project

```bash
cd your-project
akmon chat
```

The TUI opens. If this is your first session in this project,
Akmon suggests running `/init` to generate project memory.

## Step 3 — Try your first task

Type a task in plain language and press Enter:

```
explain how authentication works in this codebase
```

Akmon will search the codebase semantically, read relevant files,
and explain what it found.

## Step 4 — Make a change

```
add input validation to the create_user function
```

Before writing any file, Akmon shows a colored diff of exactly
what will change. You approve or reject each change.

## Step 5 — Generate project memory

```
/init
```

Akmon analyzes your project and generates `AKMON.md` — a context
file describing your tech stack, conventions, and architecture.
Every session after this will produce better results.

## Common first tasks

| What you want | What to type |
|---|---|
| Understand the codebase | `explain the overall architecture` |
| Find something | `find where database connections are managed` |
| Add a feature | `add rate limiting to the API endpoints` |
| Fix a bug | `the login endpoint returns 500, debug it` |
| Refactor | `refactor the user module to use the repository pattern` |
| Write tests | `add unit tests for the authentication service` |
| Review code | `review src/api.rs for security issues` |
| Analyze performance | `find the slowest database queries` |

## Key slash commands

| Command | What it does |
|---|---|
| `/help` | Show all commands |
| `/plan` | Plan before implementing |
| `/init` | Generate AKMON.md |
| `/model` | Switch model mid-session |
| `/cost` | Show session cost so far |
| `/audit` | Show audit log |
| `/clear` | Fresh context, same session |
| `/exit` | Exit with session summary |

## Exit summary

When you exit, Akmon shows a full session summary:

```
  Akmon session complete

  Session
  ──────────────────────────────────
  ID         a1b2c3d4
  Duration   12m 34s
  Directory  ~/my-project
  Model      claude-haiku-4-5 (Anthropic)

  Activity
  ──────────────────────────────────
  Messages      14
  Tool calls    23
    ✓ Succeeded  22
    ✗ Failed      1
  Files read    8
  Files written 3

  Tokens
  ──────────────────────────────────
  Input      48,291
  Output      6,847
  Cache hit  41,203 (85% savings)
  Est. cost  ~$0.047

  Audit log
  ──────────────────────────────────
  .akmon/audit/a1b2c3d4.jsonl

  Agent powering down. Goodbye!
```
