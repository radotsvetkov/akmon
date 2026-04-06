# Provider Setup

## Choosing a provider

| Provider | Best for | Approx. cost |
|---|---|---|
| Ollama | Privacy, offline work, free | Free |
| Anthropic | Highest quality | $0.80–15 per million tokens |
| OpenRouter | Model flexibility, one key | Varies by model |
| Groq | Speed, cheap inference | $0.05–0.59 per million |
| OpenAI | GPT models | $0.15–5 per million |
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

Supported Bedrock models (examples — check AWS for current list):
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
