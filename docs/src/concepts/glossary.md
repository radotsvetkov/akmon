# Glossary

Documented for Akmon `2.0.0`.

## Who this is for

Readers who want consistent terminology across Akmon tutorials, references, and review workflows.

## What you will have at the end

- Canonical meanings for Akmon terms used in docs and CI policy discussions.

## Prerequisites

- None.

## Terms

- **Session**: one Akmon run context identified by a UUID and recorded as linked events.
- **Artifact**: output file produced by a run (for example evidence JSON, audit JSONL, or `.akmon` bundle).
- **Evidence**: structured JSON artifact (`evidence.v1`) summarizing replay metadata, policy/tool outcomes, and verification context.
- **Verify**: integrity check command (`akmon verify`) that validates hash chain, object closure, and session invariants.
- **Replay**: deterministic re-execution and comparison of a recorded session (`akmon replay`) using replay modes.
- **Policy**: allow/deny control layer over tool/file/network/shell actions, including profile and pack merging.
- **Capability**: an action class available to the runtime and model through registered tools and commands.
- **Bundle**: portable `.akmon` archive containing manifest, event stream, and referenced objects for transport/import.
- **Audit log**: JSONL chain capturing auditable events for a session (`.akmon/audit/<session-id>.jsonl`).
- **Policy profile**: built-in baseline policy (`dev`, `staging`, `prod`) selectable by CLI/config.
- **Policy pack**: operator-maintained TOML/JSON policy layer merged on top of profile defaults.
- **Sentinel**: replacement object marker used by `akmon redact` to remove sensitive object bytes while preserving structure.

## Verification

Use this glossary as the canonical reference when terms differ between teams or review templates.

## Troubleshooting

- If a term is missing, check reference pages first and then update this glossary in the same PR as the feature/docs change.
