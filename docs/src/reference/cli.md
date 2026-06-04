# CLI Reference

Documented for Akmon `2.1.0`.

## Who this is for

Engineers and CI maintainers who need an accurate command surface overview before using command-specific reference pages.

## What you will have at the end

- The canonical top-level command layout.
- Common global flags and where to inspect exhaustive flags.
- Pointers to per-command reference pages for stable automation.

## Prerequisites

1. Akmon installed and runnable (`akmon --version`).
2. A project repository for interactive or headless runs.

## Steps

1. Inspect top-level help.

```bash
akmon --help
```

Expected result: all current global options and subcommands from clap output.

2. Use interactive mode (default) or headless mode (`--task`) as needed.

```bash
# Interactive TUI
akmon

# Headless run
akmon --task "run tests and summarize failures" --output json --yes
```

3. Use command-specific help for exact flags before scripting.

```bash
akmon verify --help
akmon inspect --help
akmon bundle export --help
akmon bundle import --help
akmon bundle verify --help
akmon replay --help
```

## Verification

Run a no-side-effect check on command availability:

```bash
akmon --help
akmon config --help
akmon policy --help
akmon slo --help
```

Expected result: commands parse and help exits `0`.

## Troubleshooting

- If a command in this page differs from your binary, treat `akmon --help` as source of truth.
- For provider or auth routing confusion, run `akmon config explain-provider`.
- For failed provider setup, run `akmon doctor providers`.

## Top-level subcommands (v2.0.0)

- `chat`
- `init`
- `new`
- `config`
- `doctor`
- `audit`
- `evidence`
- `slo`
- `policy`
- `scout`
- `spec`
- `import`
- `export`
- `bundle`
- `verify`
- `inspect`
- `redact`
- `diff`
- `replay`

## Common global flags

- `--task <TEXT>`: headless task run.
- `--model <MODEL>`: active model id.
- `--yes`: auto-approve read-only tools.
- `--output <text|json>`: output format.
- `--audit-log <PATH>`: override audit JSONL output path.
- `--evidence-path <PATH>`: override evidence JSON path.
- `--policy-profile <dev|staging|prod>`: select built-in policy profile.
- `--policy-pack <PATH>`: add policy pack (repeatable).
- `--policy-override <PATH>`: highest-precedence override file.
- `--web-fetch`: enable `web_fetch` tool.
- `--yes-web`: auto-approve `web_fetch` to allowed public URLs.
- `--mcp-server <URL>`: register MCP tools from remote server (repeatable).
- `--index`: load/build semantic index.
- `--plan`: read-only planning mode.
- `--architect`: two-phase planner+implementation mode.
- `--planner-model <MODEL>`: planner model override.
- `--continue`: resume last project session.
- `--session <ID_OR_PREFIX>`: resume specific session.
- `--name <TEXT>`: session display name.
- `--max-budget-usd <USD>`: headless spend cap.
- `--add-dir <DIR>`: add sandbox directory (repeatable).
- `--dossier <PATH>`: inject scout dossier context.
- `--fallback-model <MODEL>`: fallback on repeated 429/529 (headless).

## Command-specific references

- [akmon verify](./verify.md)
- [akmon inspect](./inspect.md)
- [akmon bundle export](./bundle-export.md)
- [akmon bundle import](./bundle-import.md)
- [akmon bundle verify](./bundle-verify.md)
- [akmon redact](./redact.md)
- [akmon replay](./replay.md)
- [akmon diff](./diff.md)

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
