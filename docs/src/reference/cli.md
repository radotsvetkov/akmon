# CLI reference

Documented for Akmon `3.0.0`.

An overview of every command in `akmon` 3.0.0, with links to the per-command pages. When this page and your binary disagree, `akmon <command> --help` is the source of truth.

## Evidence commands

These work on traces, journals, and bundles. None of them needs a model provider or network access.

| Command | What it does | Reference |
| --- | --- | --- |
| `akmon seal <trace.json\|session-id> --key <pk8>` | Import or select, export, sign, optionally attest, and self-verify in one step | [seal](./seal.md) |
| `akmon otel import <trace.json>` | Import an OpenTelemetry GenAI trace into a new session | [otel import](./otel-import.md) |
| `akmon bundle keygen --out <pk8>` | Generate an Ed25519 signing key | [bundle keygen](./bundle-keygen.md) |
| `akmon bundle export <session-id>` | Export a session as a `.akmon` bundle | [bundle export](./bundle-export.md) |
| `akmon bundle sign <bundle> --key <pk8>` | Sign the session head | [bundle sign](./bundle-sign.md) |
| `akmon bundle attest <bundle> --key <pk8> --operator-id <id>` | Add a signed operator attestation | [bundle attest](./bundle-attest.md) |
| `akmon bundle verify <bundle>` | Verify integrity, signatures, operators, capture level | [bundle verify](./bundle-verify.md) |
| `akmon bundle prove-openssl <bundle> --verify-key <hex>` | Write artifacts for verification with stock OpenSSL 3 | [bundle prove-openssl](./bundle-prove-openssl.md) |
| `akmon bundle import <bundle>` | Import a bundle into the local journal | [bundle import](./bundle-import.md) |
| `akmon verify <session-id>` | Verify a session in the local journal | [verify](./verify.md) |
| `akmon inspect <session-id>` | Show a session's events and contents | [inspect](./inspect.md) |
| `akmon redact <session-id> --object <hash> --reason <text> --output <bundle>` | Write a redacted derivative bundle | [redact](./redact.md) |
| `akmon diff <session-a> <session-b>` | Compare two sessions | [diff](./diff.md) |
| `akmon replay <session-id>` | Replay a full-capture session deterministically | [replay](./replay.md) |

The standalone verifier is a separate binary: [`agef-verify`](./agef-verify.md).

## Reference agent and setup commands

| Command | What it does |
| --- | --- |
| `akmon` / `akmon chat` | Interactive terminal UI for the bundled agent |
| `akmon --task "<text>"` | Headless agent run |
| `akmon init` | Generate `AKMON.md` for the current project |
| `akmon new <name>` | Create a new project with starter files |
| `akmon config …` | Manage `~/.akmon/config.toml`: `path`, `show`, `edit`, `reset`, `model`, `ollama-url`, `key`, `mcp`, `explain-provider` |
| `akmon doctor providers` | Provider health checks with remediation hints |
| `akmon policy show-effective` | Print the merged effective policy |

## Removed in 3.0.0

`akmon import`, `akmon export`, `akmon spec`, `akmon scout`, `akmon slo`, and `akmon sign` were removed, together with the `--index` and `--dossier` flags. They now fail with a usage error (exit `2`). For signing use [`akmon seal`](./seal.md) or [`akmon bundle sign`](./bundle-sign.md).

`akmon audit verify` and `akmon evidence verify` still work but are deprecated and hidden from `--help`; they print a warning. Verify sessions with `akmon bundle export <session-id>` followed by [`akmon bundle verify`](./bundle-verify.md).

## Flag placement

`--output text|json` is **not** global. It must come before the subcommand, because `seal`, `redact`, `bundle export`, `bundle sign` and `bundle attest` have their own `--output <PATH>`:

```bash
akmon --output json policy show-effective --profile staging
```

Evidence commands take their own `--format human|json` (and `markdown` for `bundle verify`) after the subcommand:

```bash
akmon bundle verify audit.akmon --format json
```

These agent flags are global and also work after a subcommand: `--audit-log`, `--evidence-path`, `--shell-allow`, `--web-fetch`, `--yes-web`, `--mcp-server`.

## Agent options

Shown under "Agent options" in `akmon --help`. They only affect the bundled agent.

- `--task <TEXT>`: headless task run.
- `--model <MODEL>`: model id (Ollama tag, or a Claude id when an Anthropic key is set).
- `--yes`: auto-approve read-only tools. Writes and `shell` still ask.
- `--audit-log <PATH>`, `--evidence-path <PATH>`: override the agent's audit and evidence output paths.
- `--policy-profile <dev|staging|prod>`, `--policy-pack <PATH>` (repeatable), `--policy-override <PATH>`.
- `--shell-allow <PATTERN>` (repeatable), `--web-fetch`, `--yes-web`, `--mcp-server <URL>` (repeatable).
- `--auto-commit`, `--plan`, `--architect`, `--planner-model <MODEL>`.
- `--continue`, `--session <ID_OR_PREFIX>`, `--name <TEXT>`.
- `--max-budget-usd <USD>`, `--fallback-model <MODEL>` (headless only), `--add-dir <DIR>` (repeatable).
- Provider flags: `--anthropic-key`, `--openrouter-key`, `--openai-key`, `--groq-key`, `--azure-endpoint`, `--azure-key`, `--azure-api-version`, `--bedrock`, `--aws-region`, `--openai-compatible-url`, `--openai-compatible-key`, `--ollama-url`.

## Setup and governance commands

### `akmon config key set <provider> [KEY]`

Store an API key. Omit `KEY` and the key is read from piped stdin, then the provider's environment variable (for example `ANTHROPIC_API_KEY`), then an interactive prompt. Passing the key on the command line works but prints a warning, because it lands in shell history. `~/.akmon/config.toml` is written with mode `0600` on unix.

```bash
printf '%s' "$ANTHROPIC_API_KEY" | akmon config key set anthropic
```

### `akmon config explain-provider`

Print how Akmon would route the current model to a provider. Explainability only; it does not change routing. Secrets are never echoed.

```bash
akmon config explain-provider
akmon config explain-provider --json
```

### `akmon doctor providers`

Provider preflight checks: key presence (masked), endpoint format and reachability, auth-mode hints. The JSON report includes the same `provider_resolution` block as `explain-provider`.

```bash
akmon doctor providers
akmon --output json doctor providers
```

Exit `0` when the active provider checks pass, `1` on a critical misconfiguration.

### `akmon policy show-effective`

Print the effective merged policy and its source layers. It honours the global `--policy-profile`, `--policy-pack`, and `--policy-override`; a conflicting profile or override between global and subcommand flags exits `2`.

```bash
akmon policy show-effective --profile staging
akmon --output json policy show-effective --policy-override ./policy.toml
```

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
- `changes[]` plus `summary` plus `risk`

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

This JSON file is the agent's legacy run summary, not signed evidence. The signed record of a run is its journal session: seal it with `akmon seal <session-id> --key signer.pk8`.
