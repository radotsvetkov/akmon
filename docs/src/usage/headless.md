# Headless mode

Headless mode is for CI and scripted runs.

## Basic run

```bash
akmon \
  --model claude-haiku-4-5-20251001 \
  --yes \
  --max-budget-usd 2.00 \
  --output json \
  --task "run cargo clippy and fix warnings"
```

Default artifacts:

- audit: `.akmon/audit/<session-id>.jsonl`
- evidence: `.akmon/evidence/<session-id>.json`

## CI governance flow

```bash
# run
akmon --yes --output json --task "run unit tests and summarize failures" | tee run.json

# verify trust artifacts
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon evidence verify .akmon/evidence/<session-id>.json

# enforce SLO policy
akmon slo verify .akmon/evidence/<session-id>.json --strict

# enforce trend regression gate
akmon slo trend .akmon/evidence/<session-id>.json \
  --baseline-dir .akmon/evidence/history \
  --window 20 \
  --strict
```

## JSON report fields

Headless JSON includes:

- lifecycle fields (`status`, `exit_reason`, `result`),
- usage/cost fields,
- additive `replay_metadata`,
- additive `reliability_metrics`.

Use `exit_reason` + command exit code for CI gating.

## Exit code guidance

- `akmon` run: process exits non-zero on runtime/config failures.
- `akmon audit verify`: `0` valid, `1` invalid/missing.
- `akmon evidence verify`: `0` valid, `1` invalid/missing.
- `akmon slo verify`: `0` pass, `1` violation, `2` invalid input/config.
- `akmon slo trend`: `0` pass, `1` violation, `2` invalid input/config.

## Common mistakes

- Running unattended jobs without `--max-budget-usd`.
- Parsing only old JSON fields and ignoring additive metrics/replay blocks.
- Using broad tasks instead of scoped, verifiable tasks.
