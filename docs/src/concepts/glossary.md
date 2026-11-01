# Glossary

Documented for Akmon `3.0.0`.

## Who this is for

Readers who want consistent terminology across Akmon tutorials, references, and review workflows.

## What you will have at the end

- Canonical meanings for Akmon terms used in docs and CI policy discussions.

## Prerequisites

- None.

## Trust and evidence terms

- **AGEF**: the Agent Evidence Format, an open format for portable AI-agent session evidence. SHA-256 content-addressed objects, a hash-linked event chain, and a portable `tar.zst` bundle. Akmon implements AGEF v0.1.3. The format is the interoperability layer; Akmon is one reference implementation. See the [AGEF specification](https://github.com/radotsvetkov/agef).
- **Bundle**: a portable `.akmon` archive (AGEF `tar.zst`) containing the manifest, the event stream, and the referenced objects, suitable for transport, import, and offline verification. A bundle is the unit you sign, ship, and verify.
- **Head**: the terminal event hash of a session, the single value that commits to the entire hash-linked chain. Because every event links to the prior one, the head fixes the whole record. Signatures are taken over the head, not over individual events.
- **Signature**: an offline Ed25519 signature over the session head, recorded in `manifest.signatures[]` (AGEF v0.1.2). The signed payload is the canonical `AGEF-SIG-v1` statement. It is produced by `akmon bundle sign` with a key from `akmon bundle keygen`, and is verifiable by `akmon bundle verify`, `agef-verify`, or plain `openssl` after `akmon bundle prove-openssl`. It answers who attested to the session, a property internal tamper-evidence alone cannot provide.
- **Operator attestation**: a separately signed `AGEF-OPERATOR-v1` claim recorded in `manifest.operator_attestations[]` (AGEF v0.1.3) that binds the session head to operator identity fields (id, display name, role, org), produced by `akmon bundle attest`. Verification attaches trust to the key, never to the self-asserted string; trust in the name is established out of band. An attested bundle verified without a trusted key reports `unverified_no_key`, which is not a failure on its own.
- **Capture level**: how much of a session's content was recorded: `full`, `partial`, or `structural`. It is part of the signed record. See [Capture levels](./capture-levels.md).
- **Full capture**: every provider request and response and every tool input and output holds real content. Sessions from the bundled reference agent, and OpenTelemetry traces with content capture enabled. Passes `--require-capture full`.
- **Partial capture**: some content slots hold content, some do not. Fails `--require-capture full`.
- **Structural capture**: no content, only the shape of the session (calls, tools, order, timing, outcomes). Typical for OpenTelemetry traces with default settings. Fails `--require-capture full`.
- **Seal**: the one-step flow `akmon seal` runs: import or select a session, export, verify, sign, optionally attest, and self-verify.
- **Session window**: the first and last event timestamps in a bundle, from the producer's clock. Covered by integrity, but not a trusted time.
- **Unsigned metadata**: manifest fields no signature covers (`created_at`, `ended_at`, producer name and version). Anyone holding the file can edit them.
- **Trusted timestamp**: an RFC 3161 token from a timestamp authority proving that data existed at a time. Not embedded in AGEF bundles yet; see [Trusted timestamps](../guides/trusted-timestamps.md).

## Runtime and review terms

- **Session**: one run context identified by a UUID and recorded as linked events.
- **Artifact**: an output file produced by a run (for example evidence JSON, audit JSONL, or a `.akmon` bundle).
- **Evidence artifact**: the bundled agent's legacy `evidence.v1` JSON run summary. Not signed; the signed evidence is the sealed `.akmon` bundle.
- **Verify**: an integrity check. `akmon verify` validates the on-disk journal hash chain and session invariants; `akmon bundle verify` and `agef-verify` validate a portable bundle, optionally its signature and operator attestation.
- **Replay**: deterministic re-execution and comparison of a recorded session (`akmon replay`). Only sessions from the bundled reference agent replay; OpenTelemetry imports are refused at any capture level.
- **Policy**: an allow and deny control layer over tool, file, network, and shell actions, including profile and pack merging.
- **Capability**: an action class available to the runtime and model through registered tools and commands.
- **Audit log**: a JSONL chain capturing auditable events for a session (`.akmon/audit/<session-id>.jsonl`).
- **Policy profile**: a built-in baseline policy (`dev`, `staging`, `prod`) selectable by CLI or config.
- **Policy pack**: an operator-maintained TOML or JSON policy layer merged on top of profile defaults.
- **Sentinel**: a replacement object marker used by `akmon redact` to remove sensitive object bytes while preserving structure.

## Verification

Use this glossary as the canonical reference when terms differ between teams or review templates.

## Troubleshooting

- If a term is missing, check the reference pages first and then update this glossary in the same PR as the feature or docs change.
