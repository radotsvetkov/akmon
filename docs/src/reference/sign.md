# akmon sign

Documented for Akmon `2.2.0`.

## Who this is for

Reviewers and operators who need an **independent, detached attestation** over a recorded
session. Akmon's journal is tamper-evident by construction (a merkle hash chain), but tamper-
evidence proves internal consistency, not provenance. A signature over the session head lets a
third party verify *who* attested to the session, the property auditors ask for when "logs from
the party being audited" are not sufficient on their own.

## What you will have at the end

- A detached signature (or transparency-log entry) over a session's head hash, produced by your
  own signing tool.

## How it works

Akmon does **not** embed a signer here. `akmon sign` reads the session's head hash from the journal
and invokes a command you configure under `[signing]` in `~/.akmon/config.toml` (Decision D-05).

> For the **native** detached-signature path (no external signing hook), use `akmon bundle sign`,
> which signs an exported bundle with an Ed25519 key made by
> [`akmon bundle keygen`](./bundle-keygen.md). `openssl genpkey` is **not** a substitute for
> `keygen`: it emits PKCS#8 v1, which `ring` rejects.

Headless runs (`akmon --task …`) invoke the same hook automatically after the session is
persisted when `[signing]` is configured. Signing is best-effort: failures are logged to stderr as
`akmon: sign (auto): …` and do not change the run's exit code. Use `akmon sign` when you need an
explicit failure exit code or JSON report.

- The command is read **only** from the trusted per-user config, never from repo-local or project
  files, so cloning a malicious repository cannot inject a command to run.
- It runs via `argv` (no shell): configured values are not word-split or shell-interpreted.
- In the configured arguments, every `{head}` and `{session_id}` token is substituted with the
  session head hash (hex) and session UUID.
- The same values are exported to the command environment as `AKMON_SESSION_HEAD` and
  `AKMON_SESSION_ID`.
- The command is terminated if it exceeds `timeout_secs` (default `60`).

## Prerequisites

- A session UUID from a completed Akmon run.
- A `[signing]` command configured in `~/.akmon/config.toml`.

## Configure the signing command

```toml
# ~/.akmon/config.toml
[signing]
command = ["/usr/local/bin/akmon-sign.sh"]   # script reads $AKMON_SESSION_HEAD
timeout_secs = 60
```

A minimal wrapper that signs the head with cosign keyless:

```bash
#!/usr/bin/env bash
# /usr/local/bin/akmon-sign.sh
set -euo pipefail
printf '%s' "$AKMON_SESSION_HEAD" \
  | cosign sign-blob --yes --output-signature "akmon-${AKMON_SESSION_ID}.sig" -
```

Or sign with GPG using token substitution instead of the environment:

```toml
[signing]
command = ["gpg", "--detach-sign", "--armor", "--output", "akmon-{session_id}.sig", "-"]
```

## Steps

1. Sign a session by UUID.

```bash
akmon sign <session-id>
```

2. Use JSON for automation.

```bash
akmon sign <session-id> --format json
```

3. Use optional flags as needed:
   - `--journal <PATH>`
   - `--format <human|json>` (default `human`)

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Signing command completed successfully |
| `1` | Signing command ran but failed (non-zero exit or timeout) |
| `2` | Usage error (no signing command configured) |
| `3` | I/O or environment error (journal/session not found, command not spawnable) |

## Verification

```bash
akmon sign <session-id> --format json | jq '.success'
```

Expected result: `true` when the configured signing command exits `0`.

## Troubleshooting

- `exit 2`: no `[signing]` command configured; add one to `~/.akmon/config.toml`.
- `exit 3`: session/journal access error or the signing executable could not be spawned; check the
  UUID, `--journal` path, and that the program in `[signing] command` exists on `PATH`.
- `exit 1`: the signing command itself failed; inspect its output, or the JSON `exit_code` /
  `timed_out` fields.

## See also

- [akmon verify](./verify.md)
- [akmon bundle keygen](./bundle-keygen.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
- [AGEF specification](https://github.com/radotsvetkov/agef)
