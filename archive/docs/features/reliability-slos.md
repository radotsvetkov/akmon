# Reliability and SLO metrics

Documented for Akmon `2.2.0`.

Akmon emits lightweight run-level reliability metrics in headless JSON output and in evidence artifacts. These are an observability signal for Akmon's own reference-agent runs. They make a run measurable in CI and operations without adding heavy tracing overhead.

These counters are observability only. They do not grant permissions and do not bypass policy enforcement. They describe how a reference-agent run behaved; they do not change what it was allowed to do. See [Security model](./security.md) for the enforcement boundary, and [Evidence artifact](./evidence.md) for where these metrics are persisted.

## Why this exists

A run that produces correct code but fails a quarter of its tool calls is not a healthy run, and a regulated pipeline needs to see that before it ships. These counters turn run health into a number CI can gate on, alongside the integrity and policy checks that the evidence artifact already carries.

## Metrics schema

`reliability_metrics` includes:

- `tool_calls_total`
- `tool_calls_success`
- `tool_calls_failure`
- `tool_latency_ms_total`
- `tool_latency_ms_avg`
- `tool_latency_ms_p95` (null when no tool calls occurred)
- `policy_denials_total`
- `retries_total`
- `timeouts_total`

## Starter SLO targets

Use these as a baseline, then tune by repo and workflow:

- tool success rate at or above 95 percent (`tool_calls_success / tool_calls_total`),
- timeout rate under 2 percent of tool calls for stable pipelines,
- a predictable policy denial ratio for your mode:
  - a higher ratio in strict or read-only modes is expected,
  - sudden spikes in implementation mode should be investigated.

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

`akmon slo trend` selects the last N valid baseline samples deterministically, then compares current metrics to baseline aggregates (median for rates, mean for totals and latency deltas).

## Scope and limitations

- `retries_total` tracks session-level continuation retries currently visible in `akmon-query`.
- `timeouts_total` tracks timeout outcomes visible in session, model, and tool paths.
- Provider-internal retry loops that are fully hidden behind provider clients are not counted separately.

## See also

- [Evidence artifact](./evidence.md)
- [Security model](./security.md)
- [Policy profiles and packs](./policy-profiles.md)
