# Configuration

Akmon reads settings from `~/.akmon/config.toml` and environment variables. CLI flags override both.

## Config wizard

```bash
akmon config
```

With **no subcommand**, Akmon runs a short interactive questionnaire on stdin: default model, optional Anthropic / OpenRouter keys, and Ollama base URL. It writes `~/.akmon/config.toml` (and may append `.akmon/` to `.gitignore` when you store an Anthropic key).

- For **automation or JSON output**, pass an explicit subcommand. `akmon config --json` **requires** a subcommand (for example `akmon config show --json`) so stdout stays machine-readable.
- Everything the wizard sets can also be configured with `akmon config <topic> …`, by editing TOML, or via **environment variables** (see [Environment variables](../reference/env-vars.md)).

## Fullscreen TUI and scrollback

The default chat UI uses the terminal’s **alternate screen**, so your emulator’s normal **scrollback may not include the full conversation**. Use the **`/transcript`** slash command to write the current chat to `.akmon/transcript_export.md` in the project, then open it in `less`, an editor, or a pager.

## Show effective config

```bash
akmon config show
```

Sensitive values are masked in output.

## `config.toml` location

| Scope | Path |
| --- | --- |
| User | `~/.akmon/config.toml` |

Project-level overrides may apply depending on your setup; use `akmon config path` to see which file is active.

## Common keys

Typical `[model]` section:

```toml
[model]
default = "claude-haiku-4-5-20251001"
anthropic_key = "sk-ant-..." # prefer: akmon config key set

[architect]
planner_model = "llama3.2"
```

For OpenRouter:

```toml
[model]
default = "anthropic/claude-haiku-4-5"
openrouter_key = "sk-or-..."
```

## Model context window and cost estimate (`[model_estimates]`)

The TUI’s **context % bar** reflects **model context window** usage from reported token counts. **Rate limits** (requests/minute, tokens/minute, spend caps) are enforced by your provider and are **independent** of that percentage—you can be low on context and still hit a limit.

Akmon’s **USD session cost** is a **rough estimate** from usage tokens and optional price tables (not a bill from your provider).

Optional rows match the **current model id** (substring match, first match wins). If you omit `context_window_tokens`, a built-in hint may still apply for common ids.

In the **TUI**, use **`/config`** (or **Ctrl+S**) → **Estimates** to edit these fields for the current model without hand-editing TOML.

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

The same table is used by the headless CLI for cost accumulation. Changes saved from the TUI apply immediately; if you edit `config.toml` by hand, restart Akmon to pick them up.

## Editor integration

Set `EDITOR` (or rely on the default) for `/edit-plan` and `/update-context` in the TUI:

```bash
export EDITOR="nvim"
```

## Local model reliability (Ollama)

Akmon now uses Ollama model metadata (when available) to tune local reliability behavior:

- adaptive first-token deadline (longer for larger local models),
- adaptive idle-stream timeout (to reduce false failures during cold starts),
- context-window hints for no-output diagnostics,
- tool-support expectation hints when a local model is likely not tool-capable.

Probe data is best-effort only. If probing fails, Akmon falls back to deterministic safe defaults and continues.

### What status hints mean

During a slow local first request, you may see status lines such as:

- `Loading <model>…`
- `Loading model into RAM… first request is slow`
- `Still loading…`

These hints are emitted consistently in both streaming and buffered response paths.

### Common local failure patterns and recovery

- **Model missing**  
  Run `ollama pull <model>` and confirm with `ollama ps`.
- **First-token timeout**  
  Warm the model once (`ollama run <model>`), then retry.
- **Idle stream timeout**  
  Check `ollama ps`; if the model process crashed/unloaded, restart it and retry.
- **No output / possible context overflow**  
  Use `/clear`, retry with smaller context, or switch to a model with larger context.
- **Tool-heavy tasks on weak local models**  
  Switch to a tool-capable local model (for example `qwen2.5-coder:7b`) if tool calls stall.

### Recommended cold-start workflow

1. Pull and warm your local model:
   - `ollama pull qwen2.5-coder:7b`
   - `ollama run qwen2.5-coder:7b`
2. Start Akmon with that model and keep early turns focused.
3. If context gets noisy, use `/clear` before retrying long tasks.

See also [Environment variables](../reference/env-vars.md) and [Configuration reference](../reference/config.md).
