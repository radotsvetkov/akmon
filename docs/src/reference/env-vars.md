# Environment Variables

Documented for Akmon `2.2.0`.

## Who this is for

Users configuring Akmon via environment (shell, CI, secret managers) instead of storing credentials in `~/.akmon/config.toml`.

## What you will have at the end

- A verified list of environment variables recognized by the current CLI/provider resolver.
- A clear provider-resolution order for debugging.

## Prerequisites

- Akmon installed and runnable (`akmon --help`).

## Steps

1. Export provider variables needed for your route.

## Provider keys

```bash
ANTHROPIC_API_KEY
OPENROUTER_API_KEY
OPENAI_API_KEY
GROQ_API_KEY
AZURE_OPENAI_ENDPOINT
AZURE_OPENAI_API_KEY
AWS_ACCESS_KEY_ID          # Bedrock
AWS_SECRET_ACCESS_KEY
AWS_SESSION_TOKEN          # optional
AWS_DEFAULT_REGION
```

2. Use CLI help to verify current env-backed flags.

```bash
akmon --help
akmon config --help
akmon doctor providers --help
```

3. Inspect effective routing decision:

```bash
akmon config explain-provider
```

## Detection order (matches `LlmConnectConfig::resolve`)

Akmon evaluates providers in a **fixed priority order** (first successful branch wins). This is **introspection-only** documentation. The runtime resolver is unchanged when you run explain commands.

1. **Amazon Bedrock** if `--bedrock` is set or `AWS_ACCESS_KEY_ID` is present (requires loadable AWS credentials including `AWS_SECRET_ACCESS_KEY`).
2. **Native Claude** (`claude-*` without `/`) via `ANTHROPIC_API_KEY` or, if absent, OpenRouter with an `anthropic/<model>` slug when `OPENROUTER_API_KEY` is set.
3. **OpenRouter** for `org/model` ids containing `/` (requires `OPENROUTER_API_KEY`).
4. **Azure OpenAI** when both `AZURE_OPENAI_ENDPOINT` and `AZURE_OPENAI_API_KEY` are set (plus `api-version`).
5. **OpenAI** when `OPENAI_API_KEY` is set and the model id matches OpenAI chat heuristics (`gpt-*`, `o1*`, …).
6. **Groq** when `GROQ_API_KEY` is set and the model id matches Groq heuristics (`llama*`, `mixtral*`).
7. **Custom OpenAI-compatible URL** when `--openai-compatible-url` (or config) is set, which requires a key for that branch.
8. **Ollama** heuristics for local-style model ids, else **Ollama** default fallback.

Use `akmon config explain-provider` to print the same order with per-branch reasons for your current model and env. Use `akmon config show` (masked) to inspect stored config.

## Additional runtime variables used by Akmon

```bash
EDITOR            # used by `akmon config edit` and TUI edit flows
AKMON_DEBUG_GIT   # enables git root discovery debug logging
```

## Wizard vs env vs `config.toml`

- **`akmon config`** (no subcommand) interactively writes `~/.akmon/config.toml`.
- The same settings usually have **environment variable** equivalents listed in the sections above (handy for CI, containers, or secret managers).
- Advanced fields (Architect defaults, `[display]`, MCP entries) are often easiest to edit in TOML or via `akmon config mcp …`; see [Configuration](../getting-started/configuration.md) and `akmon config --help`.

## Verification

```bash
akmon config show --json
akmon config explain-provider
```

Expected result: provider prerequisites are reported without printing raw secrets.

## Troubleshooting

- If Bedrock is unexpectedly selected, check whether `AWS_ACCESS_KEY_ID` is set.
- If slash model IDs fail, ensure `OPENROUTER_API_KEY` is available.
- If Azure is partially configured, set both `AZURE_OPENAI_ENDPOINT` and `AZURE_OPENAI_API_KEY`.
