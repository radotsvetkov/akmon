# Audit Log

Every Akmon session can write a **JSONL audit log** under `.akmon/audit/` for traceability and compliance-oriented workflows.

## What is logged

Each line is one JSON value. Event kinds vary by version but commonly include policy decisions, tool lifecycle, usage summaries, and errors.

Example shape (illustrative):

```json
{"timestamp":"2026-04-06T14:23:11Z","event_kind":"policy_evaluation","permission":"write_file","path":"src/main.rs","verdict":"allow","reason":"user confirmed"}
{"timestamp":"2026-04-06T14:23:15Z","event_kind":"usage","input_tokens":4821,"output_tokens":342,"cache_read_tokens":8779}
```

## Location

```
.akmon/audit/{session-id}.jsonl
```

The session id appears in the TUI status area and in the **exit summary**.

## Reading logs

```bash
# Pretty-print one file
cat .akmon/audit/<session>.jsonl | jq .

# Filter lines with jq (examples — adapt keys to your build)
cat .akmon/audit/*.jsonl | jq 'select(.verdict == "deny")'
```

## Retention

Rotate or delete like any operational log:

```bash
find .akmon/audit -mtime +30 -print
```

Add `.akmon/audit/` to **`.gitignore`** if logs should not be committed.

See [Security model](./security.md) for what is *never* written (secrets).
