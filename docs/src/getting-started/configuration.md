# Configuration

Documented for Akmon `2.0.0`.

## Who this is for

Engineers configuring Akmon for repeatable local use, CI usage, and policy/evidence-aware operation.

## What you will have at the end

- A valid `~/.akmon/config.toml`.
- A clear precedence model: CLI flags > environment > config file.
- Verified provider routing and masked config inspection commands.

## Prerequisites

1. `akmon --version` works.
2. You have either local Ollama or hosted provider credentials.
3. You can run commands in a terminal where `~/.akmon/` is writable.

## Steps

1. Run the interactive setup wizard (optional, quickest start).

```bash
akmon config
```

Expected result: Akmon writes `~/.akmon/config.toml`.

2. Inspect the effective stored config safely.

```bash
akmon config show
```

Expected result: keys are masked in output.

3. Set or update common values with explicit subcommands.

```bash
akmon config model set qwen2.5-coder:7b
akmon config ollama-url set http://localhost:11434
```

4. Verify provider resolution for the current model and environment.

```bash
akmon config explain-provider
```

Expected result: deterministic provider decision trace with candidate reasons.

5. If you manage credentials in file form, use top-level keys from `AkmonGlobalConfig`.

```toml
default_model = "qwen2.5-coder:7b"
ollama_url = "http://localhost:11434"
# anthropic_api_key = "sk-ant-..."
# openrouter_api_key = "sk-or-..."

[architect]
planner_model = "llama3.2"

[policy]
profile = "dev"
packs = [".akmon/policy-packs/team.toml"]
```

## Verification

```bash
akmon config path
akmon config show --json
akmon doctor providers
```

Expected result:
- config path resolves to `~/.akmon/config.toml`
- JSON output parses cleanly
- doctor reports either healthy provider checks or actionable failures

## Troubleshooting

- `akmon config --json` without subcommand is invalid by design; use `akmon config show --json`.
- If TUI scrollback is missing, export with `/transcript` to `.akmon/transcript_export.md`.
- If provider selection is unexpected, compare `akmon config explain-provider` with your env vars.
- Store secrets in environment variables for CI rather than committing config files.

## Model context and cost estimates (`model_estimates`)

`model_estimates` rows are optional hints for context window and rough USD estimation.

```toml
[[model_estimates]]
pattern = "haiku-4-5"
context_window_tokens = 200_000
# Optional: USD per 1M tokens (override built-in defaults when you know list pricing)
input_per_million_usd = 1.0
output_per_million_usd = 5.0
cache_read_per_million_usd = 0.1
# Shown in /context as a reminder (not enforced by Akmon)
note = "Check Anthropic console for RPM/TPM and tier limits."
```

See also [Environment variables](../reference/env-vars.md) and [Configuration reference](../reference/config.md).
