# Headless mode

Headless mode runs Akmon without interactive supervision and is ideal for CI/CD, scripted maintenance, and batch repository automation.

## When to use headless mode

Use headless mode when you need repeatable automated execution:

- pull-request checks,
- scheduled maintenance tasks,
- org-wide refactoring runs,
- scriptable JSON output for downstream tooling.

## Basic command pattern

```bash
akmon \
  --model claude-haiku-4-5-20251001 \
  --yes \
  --max-budget-usd 2.00 \
  --output json \
  --task "Run cargo clippy and fix warnings without changing behavior"
```

## Complete GitHub Actions example

```yaml
name: AI-powered lint fix

on:
  pull_request:
    types: [opened]

jobs:
  ai-fix:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Akmon
        run: |
          curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64.tar.gz | tar xz
          sudo mv akmon /usr/local/bin/

      - name: Run Akmon
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
        run: |
          akmon \
            --model claude-haiku-4-5-20251001 \
            --yes \
            --max-budget-usd 2.00 \
            --output json \
            --task "Run cargo clippy and fix warnings. Run cargo test after each change." \
          | tee akmon-result.json

      - name: Check exit
        run: |
          status=$(jq -r '.exit_reason' akmon-result.json)
          test "$status" = "completed"
```

## JSON output fields

Typical run report:

```json
{
  "session_id": "d329615d-...",
  "status": "completed",
  "exit_reason": "completed",
  "result": "Fixed 14 clippy warnings across 6 files",
  "tool_calls": 28,
  "files_written": ["src/main.rs", "src/auth.rs"],
  "usage": {
    "total_input_tokens": 145000,
    "total_output_tokens": 8200,
    "total_cache_read_tokens": 98000
  },
  "cost_usd": 0.18
}
```

Practical interpretation:

- `exit_reason` gates CI behavior,
- `files_written` can trigger selective test/deploy logic,
- `usage` and `cost_usd` feed budget reporting.

## Budget control in production

Recommended patterns:

- per-run cap: `--max-budget-usd 1.00`,
- fail CI when `exit_reason != completed`,
- aggregate daily spend via scheduled job reading JSON outputs.

Example shell budget gate:

```bash
result=$(akmon --yes --output json --max-budget-usd 1.5 --task "...")
cost=$(echo "$result" | jq -r '.cost_usd')
echo "run cost: $cost"
```

## Batch processing multiple repositories

```bash
#!/usr/bin/env bash
set -euo pipefail

repos=(
  ~/services/auth-service
  ~/services/payment-service
  ~/services/notification-service
)

for repo in "${repos[@]}"; do
  echo "Processing $repo"
  cd "$repo"
  result=$(akmon \
    --model claude-haiku-4-5-20251001 \
    --yes \
    --max-budget-usd 3.00 \
    --output json \
    --task "Update dependencies to latest compatible versions and run tests")
  echo "$result" | jq -r '"\(.session_id) \(.exit_reason) $\( .cost_usd )"'
done
```

## Common mistakes and troubleshooting

- **No budget cap:** always set `--max-budget-usd` in unattended runs.
- **Overly broad task:** split "fix everything" into narrower tasks.
- **Missing provider key in CI:** verify env injection before run.
- **Non-zero exit from incomplete runs:** parse `exit_reason`, do not assume success from process completion alone.
