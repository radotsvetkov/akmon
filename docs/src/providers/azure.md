# Azure OpenAI

Enterprise-hosted OpenAI-compatible deployments.

## Flags / env

```bash
akmon chat \
  --azure-endpoint https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT \
  --azure-key YOUR_KEY \
  --model gpt-4o
```

Environment variable names may map to `AZURE_OPENAI_*`; see [Environment variables](../reference/env-vars.md).

## Notes

- `--model` should match your **deployment** name.
- `azure_api_version` defaults are CLI-configurable.

More: [Provider setup](../getting-started/providers.md).
