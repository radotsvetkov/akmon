# Cost Transparency

Akmon surfaces **tokens**, **cache hits**, and **heuristic USD** estimates so you are not flying blind.

## TUI status bar

You typically see:

- **Tokens** — cumulative usage for the session
- **Cache** — prompt-cache read tokens (when applicable)
- **~$x.xx** — estimate from bundled pricing tables (unknown models may show `~$?`)
- **Free / local** — omitted when running local-only inference profiles

## Cache savings

Providers that support **prompt caching** (notably Anthropic-class flows) bill cached input at a fraction of fresh prompt tokens. High cache hit rates dramatically lower cost.

## `/cost` overlay

```
/cost
```

Shows a textual breakdown mid-session.

## Exit summary

On `/exit`, **Ctrl+D**, or idle **Ctrl+C**, a plaintext summary may include token totals, cache notes, estimated cost, and the audit path.

## Pricing reality

Tables are **heuristic** and lag market changes. For billing, trust your provider’s dashboard.

Provider overview: [Provider setup](../getting-started/providers.md).
