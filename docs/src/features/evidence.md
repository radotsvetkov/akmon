# Evidence Artifact

Akmon writes a deterministic evidence artifact per successful/budget-stopped headless run:

```text
.akmon/evidence/<session-id>.json
```

You can override the location with `--evidence-path <path>`.

## Why it exists

The artifact is designed for CI/PR automation and links:

- replay metadata (`replay_metadata` hashes),
- audit-chain integrity (`audit.audit_chain_valid`, `session_final_hash`),
- policy decision summary,
- tool execution timeline + aggregates,
- reliability/SLO metrics,
- touched files and verification outcomes.

## Schema version

Artifacts include:

- `evidence_schema_version` (currently `evidence.v1`)

Consumers should validate schema version before strict parsing.

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

## Validation

Use:

```bash
akmon evidence verify .akmon/evidence/<session-id>.json
```

Validation checks schema support, replay metadata shape, linked audit-chain
integrity, and session hash consistency.

Exit codes:

- `0`: evidence valid
- `1`: evidence invalid/missing/tampered

## Enforcing SLOs in CI

You can enforce reliability guardrails directly against evidence:

```bash
akmon slo verify .akmon/evidence/<session-id>.json --strict
```

Example GitHub Actions step:

```yaml
- name: Enforce Akmon SLO guardrails
  run: |
    akmon slo verify .akmon/evidence/${SESSION_ID}.json \
      --thresholds .github/akmon/slo.toml \
      --strict
```

Trend/regression check against prior evidence history:

```yaml
- name: Detect reliability regressions
  run: |
    akmon slo trend .akmon/evidence/${SESSION_ID}.json \
      --baseline-dir .akmon/evidence/history \
      --window 20 \
      --strict
```

## Policy provenance and hash impact

Evidence keeps replay metadata `policy_hash`, which is computed from the effective
runtime policy mode/config after profile/pack/local/override merge. Any change in
selected profile or pack contents deterministically changes `policy_hash`, enabling
CI/PR systems to detect policy-governance drift even when behavior changes are subtle.

## Migration note

Treat `evidence_schema_version` as required for parsers and reject unknown versions.
`reliability_metrics` is additive and stable-keyed for CI automation.
