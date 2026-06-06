# Tutorial: CI headless governance flow

Documented for Akmon `2.2.0`.

Time estimate: 25-35 minutes  
Complexity: Intermediate

## Who this is for

Platform and release teams running Akmon non-interactively in CI who want the pipeline to fail unless a session produced a signed, independently verifiable evidence bundle. The own-agent audit, evidence, and SLO checks still run, but here they feed a single artifact a reviewer or auditor can verify offline.

## What you will have at the end

- A reproducible headless run command with a budget cap.
- Own-agent integrity gates (`audit`, `evidence`, `verify`) and SLO and trend gates.
- A signed `.akmon` bundle, exported from the session and verified in CI with `akmon bundle verify --verify-key --require-signature`, so the gate that matters is provenance, not just that the agent ran.

## Prerequisites

1. CI runner has `akmon` installed.
2. Runner has provider credentials (for example `ANTHROPIC_API_KEY`) or a local model setup.
3. Repository has write access to `.akmon/` output paths.
4. A signing key is available to CI as a secret. Generate it once with `akmon bundle keygen --out signer.pk8 --public-out signer.pub.hex`, keep `signer.pk8` secret (inject it from your CI secret store at runtime), and commit or publish only `signer.pub.hex`.

## Steps

1. Execute a headless run with JSON output and a budget cap.

```bash
akmon --yes --output json \
  --max-budget-usd 2.00 \
  --task "run cargo test and summarize failures" \
  | tee run.json
```

2. Extract the session ID and run the own-agent integrity checks.

```bash
SESSION_ID="$(jq -r '.session_id' run.json)"
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
akmon verify "${SESSION_ID}"
```

3. Enforce per-run SLO thresholds and the trend gate.

```bash
akmon slo verify ".akmon/evidence/${SESSION_ID}.json" \
  --thresholds .github/akmon/slo.toml \
  --strict

akmon slo trend ".akmon/evidence/${SESSION_ID}.json" \
  --baseline-dir .akmon/evidence/history \
  --window 20 \
  --strict
```

4. Export the session as a bundle and sign it offline.

```bash
akmon bundle export "${SESSION_ID}" --output "session.akmon"
akmon bundle sign "session.akmon" --key signer.pk8
```

5. The governance gate: require a present, valid signature against the published public key.

```bash
akmon bundle verify "session.akmon" \
  --verify-key signer.pub.hex \
  --require-signature \
  --require-capture full
```

`akmon bundle verify` exits `0` only when the bundle's objects, event chain, and manifest head are internally consistent and the head signature verifies against `signer.pub.hex`. With `--require-signature`, a missing or stripped signature is a hard failure (exit `1`) rather than a quiet pass. A reference-agent run is `full` capture, so `--require-capture full` passes here; it would correctly fail on a structural OTEL import. Exit `3` indicates an I/O or environment error. This is the gate that proves a verifiable record exists, not merely that the agent finished.

6. Wire the same sequence into CI. The bundle verification step is the one that blocks the merge.

```yaml
- name: Run Akmon headless
  run: akmon --yes --output json --max-budget-usd 2.00 --task "run tests and summarize failures" | tee run.json

- name: Extract session ID
  run: echo "SESSION_ID=$(jq -r '.session_id' run.json)" >> $GITHUB_ENV

- name: Verify audit, evidence, and session integrity
  run: |
    akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
    akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
    akmon verify "${SESSION_ID}"

- name: Enforce SLO and trend guardrails
  run: |
    akmon slo verify ".akmon/evidence/${SESSION_ID}.json" --strict
    akmon slo trend ".akmon/evidence/${SESSION_ID}.json" --baseline-dir .akmon/evidence/history --window 20 --strict

- name: Export and sign the evidence bundle
  run: |
    printf '%s' "${AKMON_SIGNING_KEY_B64}" | base64 -d > signer.pk8
    akmon bundle export "${SESSION_ID}" --output session.akmon
    akmon bundle sign session.akmon --key signer.pk8
    rm -f signer.pk8

- name: Gate on a signed, verified bundle
  run: akmon bundle verify session.akmon --verify-key signer.pub.hex --require-signature --require-capture full

- name: Upload the signed evidence bundle
  uses: actions/upload-artifact@v4
  with:
    name: akmon-evidence
    path: session.akmon
```

`AKMON_SIGNING_KEY_B64` is the base64 of `signer.pk8`, stored as a CI secret. The private key is written only for the signing step and removed immediately.

## What gets recorded in evidence

- Reliability metrics used by `slo verify` and `slo trend`.
- Replay metadata hashes, including the policy and tool-registry hashes, for deterministic validation context.
- Provider resolution and session-level run status.

The same content is what the exported bundle commits to and the head signature seals, so the artifact CI uploads is exactly what a reviewer verifies later.

## How a reviewer validates this

1. Confirm all own-agent integrity commands exit `0`.
2. Confirm `akmon bundle verify session.akmon --verify-key signer.pub.hex --require-signature` exits `0` with a `verified` signature outcome.
3. Confirm `--require-capture full` passes for the reference-agent run.
4. Confirm CI artifacts include the signed `session.akmon` and `run.json` for retained runs.

A reviewer who does not run Akmon can verify the uploaded bundle with the standalone `agef-verify` or with plain `openssl`; see [Verify evidence on an air-gapped machine](../use-cases/air-gapped-audit.md).

## Verification

```bash
jq '{session_id,status,reliability_metrics}' run.json
```

Expected result: non-empty `session_id`, an explicit `status`, and a reliability metrics object.

## Troubleshooting

- If CI fails before Akmon starts, verify provider credentials in the runner environment.
- If `akmon bundle sign` rejects the key, regenerate it with `akmon bundle keygen`; `openssl genpkey` emits PKCS#8 v1, which the signing path rejects.
- If `bundle verify --require-signature` fails, the signature is missing, stripped, or does not match `signer.pub.hex`. Confirm the signing step ran and the public key matches the private key in CI.
- If `slo verify` fails, inspect the threshold file and the `violations` output.
- If policy denials block the run, inspect `policy_denials_total` in metrics and reconcile with the configured profile and packs.
- Failure behavior is intentional: non-zero exits from `audit`, `evidence`, `verify`, `slo`, and `bundle verify` should fail pipeline gates.

## See also

- [akmon bundle verify](../reference/bundle-verify.md)
- [akmon bundle sign](../reference/sign.md)
- [akmon bundle keygen](../reference/bundle-keygen.md)
- [Assemble a signed evidence pack for a regulated release](../use-cases/release-evidence-pack.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
