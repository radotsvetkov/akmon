# CLI Reference

## Synopsis

```bash
akmon [FLAGS] [SUBCOMMAND]
```

Use `akmon --help` for the authoritative flag list for your installed binary.

## Common global flags

| Flag | Purpose |
| --- | --- |
| `--task` | Headless task run (no TUI) |
| `--model` | Model id for run/session |
| `--yes` | Auto-approve read operations |
| `--web-fetch` | Enable `web_fetch` tool |
| `--yes-web` | Auto-approve web fetch where policy allows |
| `--output` | `text` or `json` |
| `--audit-log` | Override audit JSONL path |
| `--evidence-path` | Override evidence JSON path |
| `--policy-profile` | Policy profile: `dev`, `staging`, `prod` |
| `--policy-pack` | Additional policy pack file (repeatable) |
| `--policy-override` | Highest-precedence policy override file |

## Trust and governance commands

### `akmon audit verify <PATH>`

Verify tamper-evident audit chain integrity.

```bash
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon --output json audit verify .akmon/audit/<session-id>.jsonl
```

Exit codes:

- `0`: valid chain
- `1`: invalid/missing audit file

### `akmon evidence verify <PATH>`

Verify evidence schema + replay metadata shape + linked audit consistency.

```bash
akmon evidence verify .akmon/evidence/<session-id>.json
akmon --output json evidence verify .akmon/evidence/<session-id>.json
```

Exit codes:

- `0`: valid evidence
- `1`: invalid/missing evidence

### `akmon slo verify <PATH>`

Evaluate run/evidence reliability metrics against thresholds.

```bash
akmon slo verify .akmon/evidence/<session-id>.json --strict
akmon slo verify run.json --thresholds .akmon/slo.toml
akmon --output json slo verify run.json --min-tool-success-rate 0.95
```

Exit codes:

- `0`: all enabled checks pass
- `1`: threshold violation(s)
- `2`: invalid input/config

### `akmon slo trend <CURRENT_PATH>`

Compare current metrics vs historical baseline window.

```bash
akmon slo trend .akmon/evidence/current.json \
  --baseline-dir .akmon/evidence/history \
  --window 20 \
  --strict

akmon --output json slo trend run.json \
  --baseline-file .akmon/evidence/r1.json \
  --baseline-file .akmon/evidence/r2.json
```

Exit codes:

- `0`: no regression violations
- `1`: regression violations (or strict-mode skipped checks)
- `2`: invalid input/config/baseline setup

### `akmon policy show-effective`

Print effective merged configured policy and source layers.

```bash
akmon policy show-effective --profile staging
akmon policy show-effective --profile prod --policy-pack .akmon/policy-packs/org.toml
akmon --output json policy show-effective --policy-override /tmp/policy.toml
```

Exit codes:

- `0`: command succeeded (with or without configured policy sources)
- `1`: merge/load error (invalid pack, ambiguous local policy, parse failure)

## Headless JSON output shape

Example (`akmon --output json --task "..."`):

```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "completed",
  "exit_reason": "completed",
  "result": "Done",
  "tool_calls": 6,
  "files_written": ["src/main.rs"],
  "usage": {
    "total_input_tokens": 12100,
    "total_output_tokens": 830,
    "total_cache_read_tokens": 2100
  },
  "cost_usd": 0.04,
  "replay_metadata": {
    "hash_algorithm": "sha256",
    "provider_name": "ollama",
    "model_id": "llama3.2",
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "policy_hash": "...",
    "config_hash": "...",
    "tool_registry_hash": "...",
    "prompt_assembly_hash": "..."
  },
  "reliability_metrics": {
    "tool_calls_total": 6,
    "tool_calls_success": 6,
    "tool_calls_failure": 0,
    "tool_latency_ms_total": 132,
    "tool_latency_ms_avg": 22,
    "tool_latency_ms_p95": 40,
    "policy_denials_total": 0,
    "retries_total": 0,
    "timeouts_total": 0
  }
}
```

## Evidence output location

By default, headless runs write:

```text
.akmon/evidence/<session-id>.json
```

Override with:

```bash
akmon --task "..." --evidence-path /tmp/run-evidence.json
```
