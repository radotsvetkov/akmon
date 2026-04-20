# Audit log

Akmon can emit per-session JSONL audit logs for traceability, debugging, and compliance-oriented workflows.

## Why this matters

For AI-assisted development, "it changed some files" is not enough. Teams need to know:

- what the model requested,
- what the policy allowed or denied,
- what commands/files were executed,
- when and why a session stopped.

Audit logs provide this evidence trail.

## Log location

Typical path:

```text
.akmon/audit/<session-id>.jsonl
```

The session id is shown in UI/session output and links runtime behavior to log artifacts.

## Verification

```bash
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon --output json audit verify .akmon/audit/<session-id>.jsonl
```

Exit codes:

- `0`: chain valid
- `1`: invalid/missing/tampered audit file

## Typical event categories

- policy decisions (`allow`, `deny`, `prompted`),
- tool lifecycle (requested/executed/completed/failed),
- usage and cost-related summaries,
- session lifecycle transitions (start/done/error).

Each JSONL row also includes tamper-evident chain metadata:

- `schema_version` (`"audit_chain.v1"`),
- `event_index`,
- `prev_hash`,
- `event_hash`,
- optional `session_final_hash` on the final row.

Example lines:

```json
{"schema_version":"audit_chain.v1","event_index":0,"event_hash":"...","event_kind":"policy_evaluation","timestamp":"2026-04-06T14:23:11Z","permission":"write_file","path":"src/main.rs","verdict":"allow","reason":"user confirmed"}
{"schema_version":"audit_chain.v1","event_index":1,"prev_hash":"...","event_hash":"...","session_final_hash":"...","event_kind":"tool_call","timestamp":"2026-04-06T14:23:15Z","tool":"shell","args":{"command":"cargo check"},"result":"ok"}
```

Downstream parsers should deserialize each line as `AuditChainRecord` and read
the original event payload from `.event` (flattened fields like `event_kind`
remain present in JSON).

For replay workflows, pair this audit chain with CLI JSON output
`replay_metadata` hashes (`policy_hash`, `config_hash`, `tool_registry_hash`,
optional `prompt_assembly_hash`) to validate run prerequisites before replaying.

Akmon evidence artifacts (`.akmon/evidence/<session-id>.json`) include the
linked `audit_log_path`, audit validation result, and final chain hash so CI
can verify replay + audit + tool/file outcomes together.

## Migration note

If you previously parsed each line as plain `AuditEvent`, migrate to
`AuditChainRecord` and validate:

- `schema_version == "audit_chain.v1"`
- monotonic `event_index`
- `prev_hash`/`event_hash` chain integrity
- `session_final_hash` only on the final record

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

- **Missing logs:** verify audit logging is enabled in your workflow/config.
- **Unparsable lines:** use line-by-line JSON parser (`jq -c`) and detect malformed rows early.
- **Chain verification failure:** a line was modified/reordered or truncated; re-export the original audit artifact and verify with the same file bytes.
- **Secrets concern:** logs should not contain API keys; if they appear, rotate keys and report immediately.

See also [security model](./security.md) and [cost guide](./cost.md).
