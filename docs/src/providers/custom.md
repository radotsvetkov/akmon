# Custom OpenAI-compatible endpoints

Any server that speaks the OpenAI Chat Completions (or compatible) HTTP API — for example **LM Studio**, **vLLM**, **LiteLLM**, **Together**, **Mistral**, or a corporate gateway.

## CLI

```bash
akmon chat \
  --openai-compatible-url http://localhost:1234/v1 \
  --openai-compatible-key optional-if-your-proxy-needs-it \
  --model your-local-model-name
```

## Tips

- URL usually ends with `/v1` for OpenAI-style routers.
- Model string must match what the server exposes as `model`.
- TLS and auth are your responsibility (reverse proxy, VPN, etc.).

More: [Provider setup](../getting-started/providers.md).
