# Tutorial: Enterprise policy profile rollout

Documented for Akmon `2.1.0`.

Time estimate: 30-40 minutes  
Complexity: Advanced

## Who this is for

Platform/security teams introducing policy governance from developer-friendly defaults to production guardrails.

## What you will have at the end

- A staged rollout flow across `dev`, `staging`, and `prod`.
- An org policy pack with deterministic merge behavior.
- Evidence-driven checks showing what denials look like in practice.

## Prerequisites

1. Repository contains `.akmon/` directory.
2. You can run headless tasks (`akmon --task ...`).
3. Team agrees on approval and CI gate expectations.

## Steps

1. Establish baseline with built-in `dev` profile.

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

Inspect effective result:

```bash
akmon policy show-effective --profile dev --policy-pack .akmon/policy-packs/org.toml
```

3. Roll into `staging` for CI-like gating.

```bash
akmon policy show-effective --profile staging --policy-pack .akmon/policy-packs/org.toml
akmon --policy-profile staging --policy-pack .akmon/policy-packs/org.toml --yes --output json \
  --task "run non-mutating checks and summarize findings" | tee staging-run.json
```

4. Promote to `prod` and validate expected denials.

```bash
akmon policy show-effective --profile prod --policy-pack .akmon/policy-packs/org.toml
akmon --policy-profile prod --policy-pack .akmon/policy-packs/org.toml \
  --task "run shell command: cargo test"
```

Expected result: command path involving `shell` is denied by policy.

5. Confirm allowed read-heavy workflow still succeeds.

```bash
akmon --policy-profile prod --policy-pack .akmon/policy-packs/org.toml \
  --task "list auth module files and summarize"
```

Merge precedence:
`profile < packs < project-local policy < CLI override`

## What gets recorded in evidence

- Policy decision counters (`allow`/`deny`/`prompted`).
- Decision samples and replay metadata policy hash.
- Reliability metrics showing denial events in governed runs.

## How a reviewer validates this

1. Compare `akmon policy show-effective` output across profiles.
2. Confirm expected deny behavior appears for prohibited capabilities.
3. Verify governed run artifacts with `audit/evidence/verify`.

## Verification

```bash
SESSION_ID="$(jq -r '.session_id' staging-run.json)"
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
akmon verify "${SESSION_ID}"
```

## Anti-patterns

- Moving directly to `prod` without staging validation.
- Using ad hoc CLI overrides in CI without documenting governance rationale.
- Interpreting denial-heavy runs as failures without checking policy intent.

## Troubleshooting

- If policy file parsing fails, validate TOML syntax and paths.
- If effective view is empty, confirm selected profile/packs are actually passed.
