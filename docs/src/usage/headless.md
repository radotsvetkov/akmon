# Headless mode

Documented for Akmon `3.0.0`.

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

- journal session: the signed record source; seal it with `akmon seal <session-id>`
- audit (legacy): `.akmon/audit/<session-id>.jsonl`
- evidence summary (legacy): `.akmon/evidence/<session-id>.json`

## CI governance flow

```bash
# run
akmon --yes --output json --task "run unit tests and summarize failures" | tee run.json
SESSION_ID="$(jq -r '.session_id' run.json)"

# seal the session into a signed bundle and self-verify it
akmon seal "${SESSION_ID}" --key signer.pk8 --output "${SESSION_ID}.akmon"

# gate: the bundle must verify against the trusted key, at full capture
agef-verify "${SESSION_ID}.akmon" --verify-key signer.pub --require-signature --require-capture full
```

Archive the bundle and its `.pub` file as a build artifact. See [CI headless governance flow](../tutorials/ci-headless-governance.md).

## JSON report fields

Headless JSON includes:

- lifecycle fields (`status`, `exit_reason`, `result`),
- usage/cost fields,
- additive `replay_metadata`,
- additive `reliability_metrics`.

Use `exit_reason` + command exit code for CI gating.

## Exit code guidance

- `akmon` run: process exits non-zero on runtime/config failures.
- `akmon seal`: `0` sealed, `1` verification failure, `2` bad input, `3` I/O error.
- `agef-verify` / `akmon bundle verify`: `0` passed, `1` failed, `2` usage, `3` I/O.
- `akmon audit verify` and `akmon evidence verify` still work but are deprecated and hidden.

## Common mistakes

- Running unattended jobs without `--max-budget-usd`.
- Parsing only old JSON fields and ignoring additive metrics/replay blocks.
- Using broad tasks instead of scoped, verifiable tasks.
