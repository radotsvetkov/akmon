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
