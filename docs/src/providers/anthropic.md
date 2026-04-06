# Anthropic

Claude models via the Anthropic API — strong default for quality on code tasks.

## Auth

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

Or `anthropic_key` in `~/.akmon/config.toml` (prefer `akmon config key set`).

## Examples

```bash
akmon chat --model claude-haiku-4-5-20251001
akmon chat --model claude-sonnet-4-6
akmon chat --model claude-opus-4-6
```

Model ids depend on what Anthropic exposes; check their docs for current strings.

## Notes

- Prompt **caching** can reduce cost; Akmon surfaces cache read tokens in the TUI. See [Cost transparency](../features/cost.md).

More: [Provider setup](../getting-started/providers.md).
