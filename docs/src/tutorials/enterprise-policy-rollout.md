# Tutorial: Enterprise policy profile rollout

Documented for Akmon `2.2.0`.

Time estimate: 30-45 minutes  
Complexity: Advanced

## Who this is for

Platform and security teams introducing policy governance for AI agents, moving from developer-friendly defaults to production guardrails, and tying the policy that governed a run to the signed evidence a reviewer receives.

## What you will have at the end

- A staged rollout flow across `dev`, `staging`, and `prod`.
- An organizational policy pack with deterministic merge behavior.
- Evidence that records which policy governed each run, through the recorded `policy_hash`, so a reviewer can confirm the run was governed by the profile you intended.
- A signed bundle that carries that evidence, verifiable offline.

## Prerequisites

1. Repository contains an `.akmon/` directory.
2. You can run headless tasks (`akmon --task ...`).
3. Team agrees on approval and CI gate expectations.
4. A signing key for the evidence handoff, created with `akmon bundle keygen --out signer.pk8 --public-out signer.pub.hex`. Keep the private key secret and publish only the public key.

## Steps

1. Establish a baseline with the built-in `dev` profile.

```bash
akmon policy show-effective --profile dev
akmon --policy-profile dev --task "list API modules and summarize auth boundaries"
```

2. Add an organizational policy pack.

Create `.akmon/policy-packs/org.toml`:

```toml
[tools]
deny = ["shell"]

[network]
deny_domains = ["*"]
```

Inspect the effective result:

```bash
akmon policy show-effective --profile dev --policy-pack .akmon/policy-packs/org.toml
```

3. Roll into `staging` for CI-like gating.

```bash
akmon policy show-effective --profile staging --policy-pack .akmon/policy-packs/org.toml
akmon --policy-profile staging --policy-pack .akmon/policy-packs/org.toml --yes --output json \
  --task "run non-mutating checks and summarize findings" | tee staging-run.json
```

4. Promote to `prod` and validate the expected denials.

```bash
akmon policy show-effective --profile prod --policy-pack .akmon/policy-packs/org.toml
akmon --policy-profile prod --policy-pack .akmon/policy-packs/org.toml \
  --task "run shell command: cargo test"
```

Expected result: a command path involving `shell` is denied by policy.

5. Confirm an allowed read-heavy workflow still succeeds.

```bash
akmon --policy-profile prod --policy-pack .akmon/policy-packs/org.toml \
  --task "list auth module files and summarize"
```

Merge precedence:
`profile < packs < project-local policy < CLI override`

## Tie the policy to the evidence

The effective policy that governed a run is recorded in that run's evidence as a `policy_hash` in the replay metadata. This is the link between governance and proof: the same effective policy that `policy show-effective` describes is the one whose hash is committed to the evidence and, in turn, sealed by the bundle's head signature.

Inspect the recorded hash for a governed run:

```bash
SESSION_ID="$(jq -r '.session_id' staging-run.json)"
jq '.replay_metadata.policy_hash' ".akmon/evidence/${SESSION_ID}.json"
```

Because the same effective policy produces the same `policy_hash`, two governed runs under the same profile and packs commit to the same value. A reviewer can compare that hash across runs to confirm the policy did not change between them, and can match it to the profile your rollout documents as approved for that environment.

## Hand reviewers a signed bundle

Export the governed session, sign it offline, and verify the signature. The reviewer then receives a bundle whose sealed evidence includes the `policy_hash`, so the governance context travels with the proof.

```bash
akmon bundle export "${SESSION_ID}" --output "${SESSION_ID}.akmon"
akmon bundle sign "${SESSION_ID}.akmon" --key signer.pk8
akmon bundle verify "${SESSION_ID}.akmon" --verify-key signer.pub.hex --require-signature
```

A reviewer can verify this offline with `akmon bundle verify`, the standalone `agef-verify`, or plain `openssl`; see [Verify evidence on an air-gapped machine](../use-cases/air-gapped-audit.md).

## What gets recorded in evidence

- Policy decision counters (`allow`, `deny`, `prompted`).
- Decision samples and the replay-metadata `policy_hash` for the effective policy.
- Reliability metrics, including denial events in governed runs.

## How a reviewer validates this

1. Compare `akmon policy show-effective` output across profiles to confirm each environment's guardrails.
2. Confirm the expected deny behavior appears for prohibited capabilities.
3. Confirm the recorded `policy_hash` matches the profile approved for the environment, and is stable across runs that should share a policy.
4. Verify the signed bundle with `akmon bundle verify ... --require-signature`.

## Anti-patterns

- Moving directly to `prod` without staging validation.
- Using ad hoc CLI overrides in CI without documenting the governance rationale; an override changes the effective policy and therefore the `policy_hash`.
- Interpreting denial-heavy runs as failures without checking policy intent.
- Distributing an unsigned bundle when the reviewer must establish provenance.

## Troubleshooting

- If policy file parsing fails, validate TOML syntax and paths.
- If the effective view is empty, confirm the selected profile and packs are actually passed.
- If two runs you expected to share a policy show different `policy_hash` values, an override or project-local policy changed the effective merge; reconcile with the precedence order above.

## See also

- [akmon bundle verify](../reference/bundle-verify.md)
- [akmon bundle sign](../reference/sign.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
- [Compliance and evidence](../concepts/compliance.md)
