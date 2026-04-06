# Amazon Bedrock

Run Claude and other models inside AWS.

## Auth

Typical environment:

```bash
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
export AWS_DEFAULT_REGION=us-east-1
```

Use IAM roles on EC2/EKS where possible instead of long-lived keys.

## CLI

```bash
akmon chat --bedrock \
  --model anthropic.claude-haiku-4-5-v1:0
```

Supported model ids change with AWS; consult Bedrock documentation for the latest inventory.

More: [Provider setup](../getting-started/providers.md).
