# OpenRouter

One API key for many hosted models. Model ids use a `provider/model` form.

## Auth

```bash
export OPENROUTER_API_KEY=sk-or-...
```

## Examples

```bash
akmon chat --model anthropic/claude-haiku-4-5
akmon chat --model meta-llama/llama-3.3-70b-instruct
akmon chat --model deepseek/deepseek-chat
```

## Notes

- Pricing and rate limits vary per underlying model.
- Akmon’s cost estimate is heuristic when pricing tables do not list every id.

More: [Provider setup](../getting-started/providers.md).
