# Security Model

Akmon is built to make **approvals, sandboxes, and evidence** first-class.

## Sandbox

File tools operate inside the **repository root** (git-aware). Attempts to escape the sandbox fail and are audited.

## Permission levels (conceptual)

| Class | Examples | Notes |
| --- | --- | --- |
| Reads | list, read, search | Often auto-approved with `--yes` |
| Writes | write, edit, patch | User confirmation + diff preview |
| Git writes | add, commit | Confirmed per policy |
| Shell | arbitrary commands | Requires `--shell-allow` patterns |
| Network fetch | HTTP(S) | Requires `--yes-web` where applicable |

Exact behavior follows the **policy engine** for your CLI mode.

## Diff preview

File mutations show a **unified diff** before confirmation so you can reject accidental edits.

## SSRF protections

`web_fetch` blocks common private / metadata / loopback targets. This reduces prompt-injection data exfiltration via URLs.

## Secrets

API keys use hardened storage types where implemented; they must not appear in logs, debug output, or user-visible errors. **Never** paste production keys into prompts.

## Prompt injection hygiene

File bodies are framed so project content cannot trivially override system instructions (delimiter-style wrapping).

## What `--yes` does

`--yes` speeds up **reads** and safe operations; **writes and destructive git** still require explicit approval in interactive / policy configurations.

For a deeper threat model review, read the source policy and sandbox crates — this page is an operator overview.
