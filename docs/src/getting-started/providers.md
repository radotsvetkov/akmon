# Provider Setup

## Choosing a provider

| Provider | Best for | Approx. cost |
|---|---|---|
| Ollama | Privacy, offline work, free | Free |
| Anthropic | Highest quality | $0.80 to 15 per million tokens |
| OpenRouter | Model flexibility, one key | Varies by model |
| Groq | Speed, cheap inference | $0.05 to 0.59 per million |
| OpenAI | GPT models | $0.15 to 5 per million |
| Azure OpenAI | Enterprise, compliance | Same as OpenAI |
| Amazon Bedrock | AWS environments, VPC | Same as Anthropic |

## Ollama

No API key needed:

```bash
# Install from https://ollama.com
ollama pull qwen2.5-coder:7b   # recommended for code
ollama pull llama3.2            # faster, lighter
ollama pull deepseek-coder-v2   # excellent for code

akmon chat  # auto-detects Ollama
akmon chat --model qwen2.5-coder:7b  # explicit
```

## Anthropic

```bash
export ANTHROPIC_API_KEY=sk-ant-...

akmon chat --model claude-haiku-4-5-20251001  # fast, cheap
akmon chat --model claude-sonnet-4-6          # balanced
akmon chat --model claude-opus-4-6            # best quality
```

## OpenRouter

One key, 500+ models, automatic failover:

```bash
export OPENROUTER_API_KEY=sk-or-...

# Model format: "provider/model-name"
akmon chat --model anthropic/claude-haiku-4-5
akmon chat --model meta-llama/llama-3.3-70b-instruct
akmon chat --model deepseek/deepseek-chat
akmon chat --model google/gemini-2.0-flash
```

## Groq

```bash
export GROQ_API_KEY=gsk_...
akmon chat --model llama-3.3-70b-versatile
akmon chat --model llama-3.1-8b-instant   # extremely fast
```

## OpenAI

```bash
export OPENAI_API_KEY=sk-...
akmon chat --model gpt-4o
akmon chat --model gpt-4o-mini
```

## Azure OpenAI

```bash
akmon chat \
  --azure-endpoint https://your-resource.openai.azure.com/openai/deployments/your-deployment \
  --azure-key your-key \
  --model gpt-4o
```

## Amazon Bedrock

```bash
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
export AWS_DEFAULT_REGION=us-east-1

akmon chat --bedrock \
  --model anthropic.claude-haiku-4-5-v1:0
```

Supported Bedrock models (examples, check AWS for current list):
- `anthropic.claude-haiku-4-5-v1:0`
- `anthropic.claude-sonnet-4-6-v1:0`
- `anthropic.claude-opus-4-6-v1:0`
- `meta.llama3-8b-instruct-v1:0`
- `meta.llama3-70b-instruct-v1:0`

## Custom OpenAI-compatible endpoint

LM Studio, Mistral, Together AI, or any OpenAI-compatible API:

```bash
akmon chat \
  --openai-compatible-url http://localhost:1234/v1 \
  --model your-model-name
```

## Saving configuration

Use the config wizard instead of setting env vars every session:

```bash
akmon config
```

Or set in `~/.akmon/config.toml`:

```toml
[model]
default = "claude-haiku-4-5-20251001"
anthropic_key = "sk-ant-..."

# Or for OpenRouter:
# default = "anthropic/claude-haiku-4-5"
# openrouter_key = "sk-or-..."
```

Per-provider pages: [Ollama](../providers/ollama.md), [Anthropic](../providers/anthropic.md), and the rest under **Providers** in the sidebar.

## Troubleshooting flow (`akmon doctor providers` + `akmon config explain-provider`)

Routing behavior is **unchanged**. These commands only **explain** which resolver branch would win for your current `--model`, flags, and `~/.akmon/config.toml`.

### Walkthrough: “Why am I on Ollama instead of OpenAI?”

1. Show the resolution trace (text or JSON):

   ```bash
   akmon config explain-provider
   akmon config explain-provider --json
   ```

   Read `selected_provider`, then scan `candidates[]` in `priority_order` order. Each row states why a branch was skipped, matched, or would have failed (named prerequisites only, no secrets).

2. Cross-check health and endpoints:

   ```bash
   akmon doctor providers
   akmon --output json doctor providers
   ```

   The JSON report includes the same `provider_resolution` block plus reachability and masked key checks.

3. Fix the **first** issue that applies: missing env vars or flags listed under `missing_prerequisites`, Azure endpoint/key mismatch, or Ollama not running. Then re-run step 1.

### Doctor-only checklist

Run:

```bash
akmon doctor providers
```

JSON mode:

```bash
akmon --output json doctor providers
```

Use this flow:

1. Fix all `base_url`/`endpoint` sanity failures first.
2. Fix missing key/auth checks for the provider you actually run with.
3. Resolve reachability failures (network, DNS, firewall, service down).
4. Re-run doctor until active provider is healthy.

Common pitfalls flagged by doctor:

- Azure endpoint missing deployment path (`/openai/deployments/<name>/chat/completions`)
- OpenAI-compatible endpoint set without key
- OpenRouter/OpenAI key missing while model selection implies that provider
- Ollama URL valid but service unreachable (`ollama serve` not running)

## Local reliability troubleshooting (Ollama)

When local runs stall or return empty output:

1. Check server/process state first:
   - `ollama ps`
2. Warm the model before long tasks:
   - `ollama run <model>`
3. If the session has drifted to large context:
   - use `/clear`, then retry
4. If tool-heavy tasks keep stalling:
   - switch to a known tool-capable local model, for example:
   - `/model qwen2.5-coder:7b`

Akmon now emits consistent loading/status hints in both streaming and buffered paths, and timeout/no-output errors include recovery actions so operators can recover without guesswork.
