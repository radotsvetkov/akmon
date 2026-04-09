# Security model

Akmon treats side-effect control as a core system, not a UI option.

## Threat model in plain terms

The main risk is not "model output text." The risk is model-triggered side effects:

- writing files,
- running shell commands,
- accessing network resources,
- mutating git state.

Akmon addresses this with sandboxing, typed permissions, and audit logs.

## Sandbox boundaries

File operations are constrained to project boundaries. Path traversal attempts are blocked. This prevents prompt-driven writes to unrelated filesystem locations in normal operation.

## Permission classes

| Class | Typical actions | Default posture |
| --- | --- | --- |
| Read | list/read/search | easier to auto-approve (`--yes`) |
| Write | write/edit/patch | requires explicit confirmation/policy allow |
| Shell | command execution | allowlisted/confirmed paths |
| Network | web fetch/MCP-backed actions | policy-checked and traceable |
| Git mutating | add/commit/restore/etc. | confirmed or explicitly policy-approved |

## Diff-first approvals

For file changes, Akmon can present unified diffs before final approval. This gives human review at the moment side effects happen, not only at the end.

## Network and SSRF posture

`web_fetch` applies protections against common private-address and metadata endpoint abuse patterns. This reduces risk from prompt injection that tries to exfiltrate internal data.

## Secrets handling

Operational guidance:

- keep keys in environment or secured config paths,
- never paste production credentials into prompts,
- rotate credentials immediately if leakage is suspected.

## What `--yes` is and is not

`--yes` is a productivity flag, not a blanket "do anything" bypass. It primarily streamlines read-oriented operations; mutating actions remain policy-gated.

## Common mistakes and troubleshooting

- **Mistake:** enabling broad shell access in unattended workflows.
  - **Fix:** restrict with precise allow patterns.
- **Mistake:** assuming audit logs replace code review.
  - **Fix:** use logs plus normal review/CI controls.
- **Mistake:** storing sensitive logs in version control.
  - **Fix:** keep `.akmon/` artifacts out of source control unless required.
