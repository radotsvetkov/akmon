# Environment Variables

Secrets are often better as **env vars** or **`akmon config key`** than literals in `config.toml`.

## Provider keys

```bash
ANTHROPIC_API_KEY
OPENROUTER_API_KEY
OPENAI_API_KEY
GROQ_API_KEY
AZURE_OPENAI_ENDPOINT      # naming may vary; check CLI help
AZURE_OPENAI_API_KEY
AWS_ACCESS_KEY_ID          # Bedrock
AWS_SECRET_ACCESS_KEY
AWS_SESSION_TOKEN          # optional
AWS_DEFAULT_REGION
```

## Runtime behavior

```bash
AKMON_OLLAMA_URL           # default http://localhost:11434
EDITOR                     # external edits (/edit-plan, /update-context)
NO_COLOR                   # disable ANSI styling
```

## Detection order (conceptual)

Rough priority when multiple backends could apply:

1. Explicit flags (`--bedrock`, Azure endpoint, OpenAI-compat URL, …)
2. Model id hints + keys (e.g. OpenRouter ids containing `/`)
3. Vendor-specific env keys
4. Local Ollama fallback

Use `akmon config show` (masked) to see what your install resolved.

## Wizard vs env vs `config.toml`

- **`akmon config`** (no subcommand) interactively writes `~/.akmon/config.toml`.
- The same settings usually have **environment variable** equivalents listed in the sections above (handy for CI, containers, or secret managers).
- Advanced fields (Architect defaults, `[display]`, MCP entries) are often easiest to edit in TOML or via `akmon config mcp …`; see [Configuration](../getting-started/configuration.md) and `akmon config --help`.
