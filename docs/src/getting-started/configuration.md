# Configuration

Akmon reads settings from `~/.akmon/config.toml` and environment variables. CLI flags override both.

## Config wizard

```bash
akmon config
```

Opens an interactive flow for default model, keys, and paths.

## Show effective config

```bash
akmon config show
```

Sensitive values are masked in output.

## `config.toml` location

| Scope | Path |
| --- | --- |
| User | `~/.akmon/config.toml` |

Project-level overrides may apply depending on your setup; use `akmon config path` to see which file is active.

## Common keys

Typical `[model]` section:

```toml
[model]
default = "claude-haiku-4-5-20251001"
anthropic_key = "sk-ant-..." # prefer: akmon config key set

[architect]
planner_model = "llama3.2"
```

For OpenRouter:

```toml
[model]
default = "anthropic/claude-haiku-4-5"
openrouter_key = "sk-or-..."
```

## Editor integration

Set `EDITOR` (or rely on the default) for `/edit-plan` and `/update-context` in the TUI:

```bash
export EDITOR="nvim"
```

See also [Environment variables](../reference/env-vars.md) and [Configuration reference](../reference/config.md).
