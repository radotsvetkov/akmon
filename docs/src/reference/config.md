# Configuration reference

This page describes common `~/.akmon/config.toml` keys. Exact schemas may grow between releases; use `akmon config show` for your version.

## `[model]`

| Key | Meaning |
| --- | --- |
| `default` | Default model id (provider-specific string). |
| `anthropic_key` | Anthropic API key (prefer `akmon config key set`). |
| `openrouter_key` | OpenRouter API key. |
| `openai_key` | OpenAI API key. |
| `groq_key` | Groq API key. |

Prefer environment variables or `akmon config key` for secrets so they are not committed.

## `[architect]` / planner

| Key | Meaning |
| --- | --- |
| `planner_model` | Default model id for `--architect` planning phase. |

## `[[model_estimates]]`

Optional rows for **context-window %** and **rough USD cost** from usage. Each row:

| Key | Meaning |
| --- | --- |
| `pattern` | Substring matched against the **current** model id (first match wins). |
| `context_window_tokens` | Context window size in tokens for % in the TUI status bar. |
| `input_per_million_usd` | Optional USD per 1M input tokens (merges with built-in defaults if only one side is set). |
| `output_per_million_usd` | Optional USD per 1M output tokens. |
| `cache_read_per_million_usd` | Optional USD per 1M cache-read tokens. |
| `note` | Free text (e.g. rate-limit reminder); shown in `/context`. |

In the **TUI**, **`/config`** (or **Ctrl+S**) → **Estimates** edits the row for the active model and writes `~/.akmon/config.toml`.

Cost display is **not** a billing statement. **Rate limits** are not modeled in-app; set expectations with `note` or your provider’s dashboard.

See [Getting started → Configuration](../getting-started/configuration.md#model-context-window-and-cost-estimate-model_estimates).

## MCP servers

Configured as tables under `mcp` / `[[mcp.servers]]` (see [MCP](../features/mcp.md)):

```toml
[[mcp.servers]]
name = "example"
url = "https://example.com/mcp"
enabled = true
```

## Paths

Akmon resolves project root (git root), `AKMON.md`, `.akmon/plans/`, `.akmon/audit/`, and optional index paths relative to the project.

## CLI

```bash
akmon config show    # masked effective config
akmon config path    # config file location
akmon config edit    # open in editor
akmon config reset   # reset options (see help)
```

Full flag and subcommand matrix: [CLI reference](./cli.md).
