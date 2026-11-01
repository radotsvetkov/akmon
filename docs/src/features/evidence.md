# Evidence artifact (legacy run summary)

Documented for Akmon `3.0.0`.

## Who this is for

Developers, reviewers, and compliance engineers who need a portable, machine-checkable record of what a reference-agent run did, for CI gating and audit handoff.

## Where the evidence artifact sits

> **Legacy.** The `evidence.v1` JSON file is an unsigned run summary from the bundled reference agent. It is not the evidence Akmon is built around. The signed, offline-verifiable record of a run is its sealed `.akmon` bundle: `akmon seal <session-id> --key signer.pk8`. `akmon evidence verify` still works in 3.0.0 but is deprecated and hidden from `--help`.

The evidence artifact is the CI-facing summary of a reference-agent run. It is a deterministic JSON document that links the run's replay metadata, audit-chain integrity, policy decisions, tool timeline, reliability metrics, and touched files into one object a pipeline can verify in a single step.

It is distinct from the portable AGEF bundle. The evidence artifact is for in-repo CI gating. The AGEF bundle is for external, offline, signed handoff. They are complementary, and a regulated workflow usually produces both: the evidence artifact gates the merge, the signed bundle is the record a third party verifies later. See [Security model](./security.md) for the verification layer, and the audit chain it builds on in [Audit log](./audit-log.md).

This artifact comes only from Akmon's own reference agent. OpenTelemetry imports do not produce it.

## What you will have at the end

- A clear model of what Akmon records in evidence artifacts.
- How to turn the same run into a sealed bundle a third party can verify.

## Prerequisites

- A completed headless reference-agent run (`akmon --task ...`) that emitted artifacts.

## Steps

1. Run a headless session to produce evidence.

```bash
akmon --task "run tests and summarize failures" --output json --yes | tee run.json
```

2. Locate the evidence artifact path.

Akmon writes a deterministic evidence artifact per successful or budget-stopped headless run:

```text
.akmon/evidence/<session-id>.json
```

You can override the location with `--evidence-path <path>`.

3. Seal the run's journal session for external use:

```bash
SESSION_ID="$(jq -r '.session_id' run.json)"
akmon seal "${SESSION_ID}" --key signer.pk8 --output "${SESSION_ID}.akmon"
```

## Why it exists

The artifact is designed for CI and PR automation and links:

- replay metadata (`replay_metadata` hashes),
- audit-chain integrity (`audit.audit_chain_valid`, `session_final_hash`),
- the policy decision summary,
- the tool execution timeline and aggregates,
- reliability counters,
- touched files and verification outcomes.

## Schema version

Artifacts include:

- `evidence_schema_version` (currently `evidence.v1`)

Consumers should validate the schema version before strict parsing.

## Example

```json
{
  "evidence_schema_version": "evidence.v1",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "generated_at": "2026-04-20T12:34:56.000Z",
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
  "audit": {
    "audit_log_path": ".akmon/audit/550e8400-e29b-41d4-a716-446655440000.jsonl",
    "audit_chain_valid": true,
    "session_final_hash": "..."
  },
  "policy": {
    "allow": 8,
    "deny": 1,
    "prompted": 2,
    "decision_samples": ["allow:read_file:..."]
  },
  "tools": {
    "timeline": [{"name": "read_file", "success": true, "message": "ok"}],
    "total": 1,
    "success": 1,
    "failure": 0
  },
  "files_touched": ["src/main.rs"],
  "verification": {
    "outcomes": [],
    "unavailable_reason": "verification commands not collected in this run"
  },
  "reliability_metrics": {
    "tool_calls_total": 1,
    "tool_calls_success": 1,
    "tool_calls_failure": 0,
    "tool_latency_ms_total": 14,
    "tool_latency_ms_avg": 14,
    "tool_latency_ms_p95": 14,
    "policy_denials_total": 0,
    "retries_total": 0,
    "timeouts_total": 0
  },
  "notes": []
}
```

## Validation (deprecated)

`akmon evidence verify <path>` still checks schema support, replay metadata shape, the linked audit-chain integrity, and session hash consistency (exit `0` valid, `1` invalid). It is hidden from `--help` and prints a deprecation warning. Use [`akmon bundle verify`](../reference/bundle-verify.md) on a sealed bundle instead.

## From in-repo evidence to a signed, offline-verifiable bundle

The evidence artifact proves integrity to a party who can run Akmon against your repository. To hand the same session to a party who does not trust you and does not run your tools, export it as a signed AGEF bundle:

1. Generate a signing key once: `akmon bundle keygen --out signer.pk8 --public-out signer.pub`.
2. Seal the session: `akmon seal <session-id> --key signer.pk8 --output session.akmon`. Add `--operator-key operator.pk8 --operator-id you@org --role approver` to record the accountable operator.
3. The recipient verifies with `agef-verify session.akmon --verify-key signer.pub --require-signature`, or with stock `openssl` after `akmon bundle prove-openssl`.

The evidence artifact and the bundle are anchored to the same content-addressed session, so the `session_final_hash` you gated on in CI is the head the signature covers.

## Troubleshooting

- If session linkage errors appear, ensure the audit and evidence files are from the same session.

## Policy provenance and hash impact

Evidence keeps the replay metadata `policy_hash`, which is computed from the effective runtime policy mode and config after the profile, pack, project-local, and override merge. Any change in the selected profile or pack contents deterministically changes `policy_hash`, so CI and PR systems can detect policy-governance drift even when behavior changes are subtle. See [Policy profiles and packs](./policy-profiles.md).

## Migration note

Treat `evidence_schema_version` as required for parsers and reject unknown versions. `reliability_metrics` is additive and stable-keyed for CI automation.

## See also

- [Audit log](./audit-log.md)
- [Security model](./security.md)
- [akmon seal](../reference/seal.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
