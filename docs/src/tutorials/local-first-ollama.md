# Tutorial A: Local-first developer flow (Ollama)

This walkthrough uses only local inference and produces trust artifacts you can verify.

## 1) Install and configure

```bash
ollama pull qwen3.5:9b
akmon --version
akmon config
```

## 2) Start interactive work

```bash
cd your-project
akmon chat --model qwen3.5:9b --policy-profile dev
```

Prompt:

```text
Add validation to the registration handler and update tests.
```

## 3) Run equivalent headless turn for artifact capture

```bash
akmon --model qwen3.5:9b --yes --output json \
  --task "Add validation to registration handler and update tests" \
  | tee run.json
```

## 4) Verify audit and evidence

```bash
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon evidence verify .akmon/evidence/<session-id>.json
```

## 5) Inspect reliability metrics

```bash
jq '.reliability_metrics' run.json
jq '.reliability_metrics' .akmon/evidence/<session-id>.json
```

You now have a local run with verifiable chain integrity and reliability counters.
