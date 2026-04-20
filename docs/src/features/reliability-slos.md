# Reliability & SLO Metrics

Akmon emits lightweight run-level reliability metrics in headless JSON output and evidence artifacts.

## Why this exists

These counters make runs measurable in CI and operations workflows without adding heavy tracing overhead.

## Metrics schema

`reliability_metrics` includes:

- `tool_calls_total`
- `tool_calls_success`
- `tool_calls_failure`
- `tool_latency_ms_total`
- `tool_latency_ms_avg`
- `tool_latency_ms_p95` (nullable when no tool calls occurred)
- `policy_denials_total`
- `retries_total`
- `timeouts_total`

## Starter SLO targets

Use these as a baseline, then tune by repo/workflow:

- tool success rate >= 95% (`tool_calls_success / tool_calls_total`)
- timeout rate < 2% of tool calls for stable pipelines
- policy denial ratio should be predictable for your mode:
  - higher in strict/read-only modes is expected,
  - sudden spikes in implementation mode should be investigated

## CI alerting pattern

Run with JSON output:

```bash
akmon --output json --task "..." > run.json
```

Example checks with `jq`:

```bash
jq -e '
  .status == "completed"
  and (
    .reliability_metrics.tool_calls_total == 0
    or (.reliability_metrics.tool_calls_success / .reliability_metrics.tool_calls_total) >= 0.95
  )
  and .reliability_metrics.timeouts_total < 3
' run.json
```

Or enforce directly with built-in guardrails:

```bash
akmon slo verify run.json --thresholds .akmon/slo.toml --strict
```

## Trend regression detection

Guard against quality drift using historical baseline artifacts:

```bash
akmon slo trend .akmon/evidence/current.json \
  --baseline-dir .akmon/evidence/history \
  --window 20 \
  --strict
```

`akmon slo trend` selects the last N valid baseline samples deterministically, then compares
current metrics to baseline aggregates (median for rates, mean for totals/latency deltas).

## Scope and limitations

- `retries_total` tracks session-level continuation retries currently visible in `akmon-query`.
- `timeouts_total` tracks timeout outcomes visible in session/model/tool paths.
- Provider-internal retry loops that are fully hidden behind provider clients are not counted separately.
