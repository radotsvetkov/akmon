# Tutorial: CI headless governance flow

Documented for Akmon `2.0.0`.

Time estimate: 20-30 minutes  
Complexity: Intermediate

## Who this is for

Teams running Akmon non-interactively in CI with explicit budget and reliability guardrails.

## What you will have at the end

- A reproducible headless run command.
- Integrity verification gates (`audit`, `evidence`, `verify`).
- SLO and trend checks that can fail CI on policy or reliability regressions.

## Prerequisites

1. CI runner has `akmon` installed.
2. Runner has provider credentials (for example `ANTHROPIC_API_KEY`) or local model setup.
3. Repository has write access to `.akmon/` output paths.

## Steps

1. Execute a headless run with JSON output and budget cap.

```bash
akmon --yes --output json \
  --max-budget-usd 2.00 \
  --task "run cargo test and summarize failures" \
  | tee run.json
```

2. Extract session ID and run integrity checks.

```bash
SESSION_ID="$(jq -r '.session_id' run.json)"
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
akmon verify "${SESSION_ID}"
```

3. Enforce per-run SLO thresholds.

```bash
akmon slo verify ".akmon/evidence/${SESSION_ID}.json" \
  --thresholds .github/akmon/slo.toml \
  --strict
```

4. Enforce trend gate against historical baseline.

```bash
akmon slo trend ".akmon/evidence/${SESSION_ID}.json" \
  --baseline-dir .akmon/evidence/history \
  --window 20 \
  --strict
```

5. Wire the same sequence into CI.

```yaml
- name: Run Akmon headless
  run: akmon --yes --output json --task "run tests and summarize failures" | tee run.json

- name: Extract session ID
  run: echo "SESSION_ID=$(jq -r '.session_id' run.json)" >> $GITHUB_ENV

- name: Verify audit, evidence, and session integrity
  run: |
    akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
    akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
    akmon verify "${SESSION_ID}"

- name: Enforce SLO and trend guardrails
  run: |
    akmon slo verify ".akmon/evidence/${SESSION_ID}.json" --strict
    akmon slo trend ".akmon/evidence/${SESSION_ID}.json" --baseline-dir .akmon/evidence/history --window 20 --strict
```

## What gets recorded in evidence

- Reliability metrics used by `slo verify` and `slo trend`.
- Replay metadata hashes for deterministic validation context.
- Provider resolution and session-level run status.

## How a reviewer validates this

1. Confirm all integrity commands exit `0`.
2. Confirm SLO/trend gates produce pass/fail outputs matching policy thresholds.
3. Confirm CI artifacts include `run.json` and evidence files for retained runs.

## Verification

```bash
jq '{session_id,status,reliability_metrics}' run.json
```

Expected result: non-empty `session_id`, explicit `status`, and reliability metrics object.

## Troubleshooting

- If CI fails before Akmon starts, verify provider credentials in runner environment.
- If `slo verify` fails, inspect threshold file and `violations` output.
- If policy denies block the run, inspect `policy_denials_total` in metrics and reconcile with configured profile/packs.
- Failure behavior is intentional: non-zero exits from `audit/evidence/verify/slo` should fail pipeline gates.
