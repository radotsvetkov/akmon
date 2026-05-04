# CLI Reference

## Synopsis

```bash
akmon [FLAGS] [SUBCOMMAND]
```

Use `akmon --help` for the authoritative flag list for your installed binary.

Beginning with v2.0, each command has its own reference page in this directory. See:

- [akmon verify](./verify.md)
- [akmon inspect](./inspect.md)

Future v2.0 commands (`export`, `import`, `redact`) will be documented in their own pages as they ship.

The sections below document v1.x commands that may be retained, retired, or migrated as part of v2.0's akmon-core retirement work (Item 6.10). Refer to those sections for current behavior until the migration completes.

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
| `--dossier` | Inject scout dossier context into prompt |

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

### `akmon config explain-provider`

Print a **deterministic provider resolution trace** for the effective CLI model and merged `~/.akmon/config.toml`. This command is **explainability only**: it does not change routing rules and mirrors the same selection as `LlmConnectConfig::resolve`.

```bash
akmon config explain-provider
akmon config explain-provider --json
akmon --output json config explain-provider
```

The JSON object includes `selected_provider`, `selected_reason`, `model_id`, optional `resolution_error`, and `candidates[]` (each with `provider`, `eligible`, `reason`, `missing_prerequisites`, `priority_order`). Secrets are never echoed—only named prerequisites.

Pair this with `akmon doctor providers` when debugging: **explain-provider** answers “which branch won and why,” while **doctor** checks reachability and credential sanity.

### `akmon doctor providers`

Run provider preflight diagnostics with actionable remediation hints.

```bash
akmon doctor providers
akmon --output json doctor providers
```

The report includes a `provider_resolution` block (same schema as `akmon config explain-provider`) so you can correlate routing decisions with health checks in one JSON payload.

Checks include:

- key/env presence (masked),
- endpoint format sanity,
- endpoint reachability (where applicable),
- auth mode mismatch hints,
- model hint availability probes where feasible.

Exit codes:

- `0`: active/required provider health checks passed
- `1`: critical misconfiguration or unreachable required provider

### `akmon scout --task "..."`

Run bounded, read-only repository scouting and write a structured dossier.

```bash
akmon scout --task "find MCP policy enforcement path"
akmon scout --task "TUI state boundaries" --max-files 300 --out .akmon/context/tui-scout.json
akmon --output json scout --task "docs CI checks"
```

Key flags:

- `--task`: required scout question.
- `--max-files`: upper bound for scanned files (default `200`).
- `--out`: dossier output path (default `.akmon/context/scout-<timestamp>.json`).
- `--max-budget-usd`: optional cap (scout itself has zero model spend).

Exit codes:

- `0`: dossier generated and written successfully
- `1`: scan or write failure
- `2`: invalid input (empty task, invalid bounds, invalid budget)

### `--dossier <PATH>` ingestion

Use a previously generated dossier to seed implementation context:

```bash
akmon scout --task "provider routing and doctor coverage" --out .akmon/context/providers.json
akmon --dossier .akmon/context/providers.json --task "implement provider explainability"
```

Invalid or malformed dossier files fail fast before session start.

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
  },
  "provider_resolution": {
    "selected_provider": "ollama",
    "selected_reason": "Resolution succeeded: selected provider `ollama` (same outcome as `LlmConnectConfig::resolve`).",
    "model_id": "llama3.2",
    "candidates": [
      {
        "provider": "bedrock",
        "eligible": false,
        "reason": "Skipped: Bedrock is considered only when `--bedrock` is set or `AWS_ACCESS_KEY_ID` is present.",
        "priority_order": 1
      }
    ]
  }
}
```

The `provider_resolution` field is additive (automation may ignore it). When present, `candidates` lists every resolver branch in priority order with human-readable reasons; it is safe to log (no secret values).

## Tool output parsing notes

When a run executes file-modifying tools (`write_file`, `edit`, `patch`, `apply_patch`), successful tool outputs are JSON strings that include a `file_change_set` payload:

- `type: "file_change_set"`
- `mode: "applied"` or `mode: "dry_run"`
- `changes[]` + `summary` + `risk`

CI consumers should parse `changes[]` as canonical and may continue accepting `files[]` as a backward-compatible alias.

## Evidence output location

By default, headless runs write:

```text
.akmon/evidence/<session-id>.json
```

Override with:

```bash
akmon --task "..." --evidence-path /tmp/run-evidence.json
```
