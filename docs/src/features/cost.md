# Cost guide

Akmon is explicit about token usage and estimated spend so you can manage AI work as an engineering budget, not a surprise invoice.

## What actually drives costs

For coding agents, the largest cost driver is usually cumulative **input** tokens, not output tokens. Each model call resends core context plus recent conversation history.

Real session example:

- 35 API calls
- 672k input tokens
- 35k output tokens
- 258k cache-read tokens
- total around $0.68

Using Haiku rates:

- input: 672000 * $0.80 / 1M = $0.5376
- output: 35000 * $4.00 / 1M = $0.1400
- cache reads: 258000 * $0.08 / 1M = $0.0206

## Prompt caching and why it matters

Cached prompt reads are much cheaper than fresh prompt tokens. Akmon surfaces cache usage in the footer and session summary so you can see when repeated context is becoming efficient.

Interpretation:

- high cache read ratio often means repeated shared context is being billed at discount rates,
- low cache ratio with high input often indicates noisy/volatile context.

## Cost by task type

| Task | Model | Typical cost | Notes |
| --- | --- | --- | --- |
| Single-file edit | Haiku | $0.01-$0.03 | few turns |
| 3-5 file feature | Haiku | $0.05-$0.20 | moderate context |
| Build small app from scratch | Haiku | $0.30-$0.80 | many turns |
| Complex refactor | Haiku | $0.20-$0.50 | exploration heavy |
| Architecture design | Sonnet | $0.50-$2.00 | stronger reasoning |

## Model selection strategy

- **Haiku:** default for most implementation work.
- **Sonnet:** architecture and hard reasoning spikes.
- **GPT-4o-mini:** strong budget option if OpenAI is preferred.
- **Ollama local models:** free token cost, but lower capability and potentially higher latency.

For local models, "free token cost" still carries operational tradeoffs:

- cold-start latency can be significant on first request,
- smaller local context windows can trigger no-output/context-overflow failure modes,
- tool-calling reliability varies by model family.

Use Akmon's local status hints and remediation guidance (`/clear`, `ollama ps`, model switch) to recover quickly.

## Practical cost controls

Use multiple levers together:

- `--max-budget-usd` for hard stop,
- plan/spec workflow to avoid repeated exploratory context,
- smaller focused tasks,
- context hygiene (`/clear` when a session gets noisy),
- use `/context` and `/cost` during long runs.

For automated runs, pair budget caps with evidence/SLO checks:

```bash
akmon --yes --output json --max-budget-usd 1.50 --task "..." | tee run.json
akmon slo verify run.json --thresholds .akmon/slo.toml
```

## Common mistakes and troubleshooting

- **Mistake:** using premium models for trivial edits.
- **Mistake:** allowing sessions to drift into repeated read loops.
- **Mistake:** ignoring cache/read metrics and only watching final cost.
- **Fix:** split work by phase and use cheaper models for discovery.
