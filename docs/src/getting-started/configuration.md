# Configuration

Akmon reads settings from `~/.akmon/config.toml` and environment variables. CLI flags override both.

## Config wizard

```bash
akmon config
```

With **no subcommand**, Akmon runs a short interactive questionnaire on stdin: default model, optional Anthropic / OpenRouter keys, and Ollama base URL. It writes `~/.akmon/config.toml` (and may append `.akmon/` to `.gitignore` when you store an Anthropic key).

- For **automation or JSON output**, pass an explicit subcommand. `akmon config --json` **requires** a subcommand (for example `akmon config show --json`) so stdout stays machine-readable.
- Everything the wizard sets can also be configured with `akmon config <topic> …`, by editing TOML, or via **environment variables** (see [Environment variables](../reference/env-vars.md)).

## Fullscreen TUI and scrollback

The default chat UI uses the terminal’s **alternate screen**, so your emulator’s normal **scrollback may not include the full conversation**. Use the **`/transcript`** slash command to write the current chat to `.akmon/transcript_export.md` in the project, then open it in `less`, an editor, or a pager.

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
