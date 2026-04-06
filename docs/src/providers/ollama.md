# Ollama (local)

Free, offline-capable inference on your machine. No API key required.

## Setup

Install [Ollama](https://ollama.com), then:

```bash
ollama pull qwen2.5-coder:7b
# or: llama3.2, deepseek-coder-v2, etc.
```

## Akmon

```bash
akmon chat
akmon chat --model qwen2.5-coder:7b
```

Override base URL if needed (see [Environment variables](../reference/env-vars.md) for `AKMON_OLLAMA_URL`).

## When to use

- Privacy-sensitive code
- No cloud spend
- Air-gapped or flaky networks

See also [Provider setup](../getting-started/providers.md).
