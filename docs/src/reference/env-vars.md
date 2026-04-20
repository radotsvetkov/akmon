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

## Detection order (matches `LlmConnectConfig::resolve`)

Akmon evaluates providers in a **fixed priority order** (first successful branch wins). This is **introspection-only** documentation—the runtime resolver is unchanged when you run explain commands.

1. **Amazon Bedrock** if `--bedrock` is set or `AWS_ACCESS_KEY_ID` is present (requires loadable AWS credentials including `AWS_SECRET_ACCESS_KEY`).
2. **Native Claude** (`claude-*` without `/`) via `ANTHROPIC_API_KEY` or, if absent, OpenRouter with an `anthropic/<model>` slug when `OPENROUTER_API_KEY` is set.
3. **OpenRouter** for `org/model` ids containing `/` (requires `OPENROUTER_API_KEY`).
4. **Azure OpenAI** when both `AZURE_OPENAI_ENDPOINT` and `AZURE_OPENAI_API_KEY` are set (plus `api-version`).
5. **OpenAI** when `OPENAI_API_KEY` is set and the model id matches OpenAI chat heuristics (`gpt-*`, `o1*`, …).
6. **Groq** when `GROQ_API_KEY` is set and the model id matches Groq heuristics (`llama*`, `mixtral*`).
7. **Custom OpenAI-compatible URL** when `--openai-compatible-url` (or config) is set—requires a key for that branch.
8. **Ollama** heuristics for local-style model ids, else **Ollama** default fallback.

Use `akmon config explain-provider` to print the same order with per-branch reasons for your current model and env. Use `akmon config show` (masked) to inspect stored config.

## Wizard vs env vs `config.toml`

- **`akmon config`** (no subcommand) interactively writes `~/.akmon/config.toml`.
- The same settings usually have **environment variable** equivalents listed in the sections above (handy for CI, containers, or secret managers).
- Advanced fields (Architect defaults, `[display]`, MCP entries) are often easiest to edit in TOML or via `akmon config mcp …`; see [Configuration](../getting-started/configuration.md) and `akmon config --help`.
