# Audit log

Documented for Akmon `2.2.0`.

The audit log is the tamper-evident decision trail of a reference-agent session. When Akmon's own bundled agent runs, it writes a per-session JSONL audit log where each row is hash-linked to the row before it. This is part of what earns a reference-agent session its `full` capture level: every policy decision and tool outcome is recorded in order, and the order cannot be changed after the fact without detection.

This is a producer-side artifact of the reference agent. An OpenTelemetry import carries whatever the third-party trace emitted at `structural` capture level, not this chain.

## Why this matters

When an AI agent changes something, "it changed some files" is not enough to put in front of a reviewer or an auditor. The audit log answers the operational questions:

- what the model requested,
- what the policy allowed, denied, or prompted on,
- what commands and files were executed,
- when and why a session stopped.

The hash chain adds one more property on top: a third party can confirm the log was not edited or reordered after the run.

## Log location

Typical path:

```text
.akmon/audit/<session-id>.jsonl
```

The session id is shown in UI and session output, and links runtime behavior to log artifacts.

## Verification

```bash
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon --output json audit verify .akmon/audit/<session-id>.jsonl
```

Exit codes:

- `0`: chain valid
- `1`: invalid, missing, or tampered audit file

## Typical event categories

- policy decisions (`allow`, `deny`, `prompted`),
- tool lifecycle (requested, executed, completed, failed),
- usage and cost-related summaries,
- session lifecycle transitions (start, done, error).

Each JSONL row also includes tamper-evident chain metadata:

- `schema_version` (`"audit_chain.v1"`),
- `event_index`,
- `prev_hash`,
- `event_hash`,
- an optional `session_final_hash` on the final row.

Example lines:

```json
{"schema_version":"audit_chain.v1","event_index":0,"event_hash":"...","event_kind":"policy_evaluation","timestamp":"2026-04-06T14:23:11Z","permission":"write_file","path":"src/main.rs","verdict":"allow","reason":"user confirmed"}
{"schema_version":"audit_chain.v1","event_index":1,"prev_hash":"...","event_hash":"...","session_final_hash":"...","event_kind":"tool_call","timestamp":"2026-04-06T14:23:15Z","tool":"shell","args":{"command":"cargo check"},"result":"ok"}
```

Downstream parsers should deserialize each line as `AuditChainRecord` and read the original event payload from `.event` (flattened fields like `event_kind` remain present in JSON).

## How the audit log relates to the verification layer

The audit chain is the runtime ledger. The evidence artifact and the AGEF bundle are the portable, signable records built on top of it.

- For replay workflows, pair this audit chain with the CLI JSON `replay_metadata` hashes (`policy_hash`, `config_hash`, `tool_registry_hash`, and the optional `prompt_assembly_hash`) to validate run prerequisites before replaying.
- Akmon [evidence artifacts](./evidence.md) (`.akmon/evidence/<session-id>.json`) include the linked `audit_log_path`, the audit validation result, and the final chain hash, so CI can verify replay, audit, and tool/file outcomes together.
- A signed AGEF bundle then carries the whole session head under an offline Ed25519 signature, so the integrity the audit chain establishes locally can be checked by a third party offline. See [Evidence artifact](./evidence.md) and [Security model](./security.md).

## Migration note

If you previously parsed each line as a plain `AuditEvent`, migrate to `AuditChainRecord` and validate:

- `schema_version == "audit_chain.v1"`,
- monotonic `event_index`,
- `prev_hash`/`event_hash` chain integrity,
- `session_final_hash` only on the final record.

## Useful queries

```bash
# show only denied actions
jq 'select(.verdict? == "deny")' .akmon/audit/*.jsonl

# list all file-write decisions
jq 'select(.permission? == "write_file")' .akmon/audit/*.jsonl
```

## Retention and operations

- treat audit logs as operational artifacts,
- rotate or archive old logs,
- avoid committing logs to git unless policy requires it.

Example retention sweep:

```bash
find .akmon/audit -type f -mtime +30 -delete
```

## Common mistakes and troubleshooting

- Missing logs: confirm audit logging is enabled in your workflow or config, and that the run was a reference-agent session (OTEL imports do not produce this chain).
- Unparsable lines: use a line-by-line JSON parser (`jq -c`) and detect malformed rows early.
- Chain verification failure: a line was modified, reordered, or truncated. Re-export the original audit artifact and verify with the same file bytes.
- Secrets concern: logs should not contain API keys. If they appear, rotate keys and report immediately.

See also [Security model](./security.md), [Evidence artifact](./evidence.md), and [Cost transparency](./cost.md).
