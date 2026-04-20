# Tutorial B: CI headless governance flow

This tutorial shows a CI-style run with evidence generation and hard pass/fail guardrails.

## 1) Headless run with JSON output

```bash
akmon --yes --output json \
  --max-budget-usd 2.00 \
  --task "run cargo test and summarize failures" \
  | tee run.json
```

## 2) Verify trust artifacts

```bash
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon evidence verify .akmon/evidence/<session-id>.json
```

## 3) Enforce per-run SLO thresholds

```bash
akmon slo verify .akmon/evidence/<session-id>.json \
  --thresholds .github/akmon/slo.toml \
  --strict
```

## 4) Enforce trend gate against baseline history

```bash
akmon slo trend .akmon/evidence/<session-id>.json \
  --baseline-dir .akmon/evidence/history \
  --window 20 \
  --strict
```

## 5) Example GitHub Actions gate

```yaml
- name: Run Akmon headless
  run: akmon --yes --output json --task "run tests and summarize" | tee run.json

- name: Verify evidence
  run: akmon evidence verify .akmon/evidence/${SESSION_ID}.json

- name: Enforce SLO
  run: akmon slo verify .akmon/evidence/${SESSION_ID}.json --strict

- name: Enforce trend regression guard
  run: akmon slo trend .akmon/evidence/${SESSION_ID}.json --baseline-dir .akmon/evidence/history --window 20 --strict
```

`slo verify`/`slo trend` non-zero exits can fail the workflow by design.
