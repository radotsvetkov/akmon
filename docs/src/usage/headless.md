# Headless Mode

Run Akmon from scripts, CI pipelines, or shell one-liners.

```bash
akmon --task "your task here" [flags]
```

## Basic usage

```bash
# Simple task
akmon --yes --task "list all TODO comments in the codebase"

# With a specific model
ANTHROPIC_API_KEY=key akmon --yes \
  --model claude-haiku-4-5-20251001 \
  --task "add error handling to src/api.rs"

# JSON output for scripting
akmon --yes --output json \
  --task "count functions in each file" \
  | jq '.result'
```

## Auto-approval flags

```bash
# --yes approves read operations automatically
# File writes still prompt for confirmation (TTY) or follow policy in CI
akmon --yes --task "analyze the codebase structure"

# --auto-commit commits each approved write (when git integration is used)
akmon --yes --auto-commit \
  --task "add rustdoc comments to all public functions in scope"
```

## Common CI patterns

```bash
# Summarize test failures
akmon --yes --output json \
  --task "run cargo test and summarize any failures" \
  | jq -r '.result'

# Code review
akmon --yes \
  --task "review the changes in git diff HEAD~1 \
          and identify any security issues"

# Documentation generation
akmon --yes --auto-commit \
  --task "add doc comments to all public items \
          that are missing them"

# Dependency audit
akmon --yes --output json \
  --task "run cargo audit and explain any vulnerabilities" \
  | jq -r '.result'
```

## Shell allow list

By default, shell commands require explicit allow patterns:

```bash
# Allow only cargo commands
akmon --yes \
  --shell-allow "cargo *" \
  --task "run tests and fix any compilation errors"

# Allow cargo and git
akmon --yes \
  --shell-allow "cargo *" \
  --shell-allow "git *" \
  --task "run tests, fix failures, and commit changes"
```

## JSON output format

Shape is versioned; approximate example:

```json
{
  "task": "count TODO comments",
  "result": "Found 14 TODO comments across 8 files...",
  "session_id": "a1b2c3d4-...",
  "tokens": {
    "input": 12847,
    "output": 423,
    "cache_read": 8200
  },
  "cost_usd": 0.012,
  "tool_calls": 6,
  "files_read": ["src/main.rs", "src/lib.rs"],
  "files_written": []
}
```

Use `akmon --help` and [CLI reference](../reference/cli.md) for the exact fields your build prints.
