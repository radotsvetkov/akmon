# Tutorial: CI headless governance flow

Documented for Akmon `3.0.0`.

Time estimate: 20-30 minutes  
Complexity: Intermediate

## Who this is for

Platform and release teams running the bundled agent non-interactively in CI who want the pipeline to fail unless the run produced a signed bundle that verifies against the team's published key.

## What you will have at the end

- A reproducible headless run with a budget cap.
- A sealed `.akmon` bundle per run, self-verified by `akmon seal` and gated by `agef-verify --require-signature`.
- The bundle and public key uploaded as build artifacts a reviewer can verify offline.

## Prerequisites

1. The CI runner has `akmon` and `agef-verify` installed.
2. Provider credentials (for example `ANTHROPIC_API_KEY`) or a local model.
3. A signing key stored as a CI secret. Generate it once with `akmon bundle keygen --out signer.pk8 --public-out signer.pub`, store `signer.pk8` in your secret store, and commit or publish `signer.pub`.

## Steps

1. Run the agent headless with JSON output and a budget cap.

```bash
akmon --yes --output json \
  --max-budget-usd 2.00 \
  --task "run cargo test and summarize failures" \
  | tee run.json
SESSION_ID="$(jq -r '.session_id' run.json)"
```

2. Check the session in the local journal.

```bash
akmon verify "${SESSION_ID}"
```

3. Seal the session. `akmon seal` exports, verifies, signs, and verifies again with the derived public key; it exits non-zero if any step fails.

```bash
akmon seal "${SESSION_ID}" --key signer.pk8 --output session.akmon --format json
```

4. The governance gate: verify against the **published** key, not the one `seal` just wrote, so a wrong or swapped CI key fails the build.

```bash
agef-verify session.akmon --verify-key signer.pub --require-signature --require-capture full
```

Exit `0` means the bundle is intact, signed by the trusted key, and a full recording. A missing or stripped signature is a hard failure (exit `1`). Exit `3` is an I/O or environment error.

5. Wire it into CI.

```yaml
- name: Run Akmon headless
  run: akmon --yes --output json --max-budget-usd 2.00 --task "run tests and summarize failures" | tee run.json

- name: Extract session ID
  run: echo "SESSION_ID=$(jq -r '.session_id' run.json)" >> $GITHUB_ENV

- name: Seal the session
  run: |
    printf '%s' "${AKMON_SIGNING_KEY_B64}" | base64 -d > signer.pk8
    akmon seal "${SESSION_ID}" --key signer.pk8 --output session.akmon
    rm -f signer.pk8

- name: Gate on a signed, verified bundle
  run: agef-verify session.akmon --verify-key signer.pub --require-signature --require-capture full

- name: Upload the evidence
  uses: actions/upload-artifact@v4
  with:
    name: akmon-evidence
    path: |
      session.akmon
      session.akmon.pub
      run.json
```

`AKMON_SIGNING_KEY_B64` is the base64 of `signer.pk8`, stored as a CI secret. The private key exists on disk only for the sealing step.

CI artifact storage usually expires after a few weeks or months. If the bundles are evidence you must retain, copy them to immutable storage ([Retention and archiving](../guides/retention.md)).

## How a reviewer validates this

1. Download `session.akmon` and obtain `signer.pub` from your published location, not from the artifact.
2. Run `agef-verify session.akmon --verify-key signer.pub --require-signature --format markdown > report.md`.
3. Confirm the verdict is PASSED and the capture level is `FULL`.

See [Verify evidence on an air-gapped machine](../use-cases/air-gapped-audit.md).

## Troubleshooting

- If CI fails before Akmon starts, check provider credentials in the runner environment.
- If `akmon seal` rejects the key, regenerate it with `akmon bundle keygen`; `openssl genpkey` emits PKCS#8 v1, which Akmon rejects.
- If `agef-verify --require-signature` fails but `seal` succeeded, the CI secret does not match the published `signer.pub`.
- If policy denials block the run, inspect the run report and reconcile with the configured profile and packs.

## See also

- [akmon seal](../reference/seal.md)
- [agef-verify](../reference/agef-verify.md)
- [Assemble a signed evidence pack for a release](../use-cases/release-evidence-pack.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
