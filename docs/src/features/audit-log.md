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

## Typical event categories

- policy decisions (`allow`, `deny`, `prompted`),
- tool lifecycle (requested/executed/completed/failed),
- usage and cost-related summaries,
- session lifecycle transitions (start/done/error).

Example lines:

```json
{"timestamp":"2026-04-06T14:23:11Z","event_kind":"policy_evaluation","permission":"write_file","path":"src/main.rs","verdict":"allow","reason":"user confirmed"}
{"timestamp":"2026-04-06T14:23:15Z","event_kind":"tool_call","tool":"shell","args":{"command":"cargo check"},"result":"ok"}
```

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
- **Secrets concern:** logs should not contain API keys; if they appear, rotate keys and report immediately.

See also [security model](./security.md) and [cost guide](./cost.md).
