# Tutorial: Example projects for regulated teams

Documented for Akmon `2.0.0`.

Time estimate: 25-35 minutes  
Complexity: Intermediate

## Who this is for

Teams validating Akmon in greenfield repositories before production rollout.

## What you will have at the end

- One reproducible starter run per stack.
- Session evidence and audit artifacts suitable for reviewer checks.
- A baseline pattern you can adapt to internal policies.

## Prerequisites

1. `akmon --version` works.
2. Provider configuration is already validated (`akmon doctor providers`).
3. You can create local git repositories.

## Steps

1. Run one starter scenario (Rust, Python, or TypeScript).

### Rust API starter (illustrative medical-device backend context)

Constraints:
- Data boundary: no external network calls beyond approved dependencies.
- Review requirement: all generated handlers must have tests.

```bash
mkdir -p ~/projects/rust-api && cd ~/projects/rust-api
git init
akmon --yes --output json --task "create an Axum REST API skeleton with tests" | tee run.json
```

### Python FastAPI starter (illustrative fintech controls context)

Constraints:
- Approval requirement: write operations must remain explicit (`--yes` only auto-approves read-only tools).
- CI requirement: pytest must be runnable.

```bash
mkdir -p ~/projects/fastapi-service && cd ~/projects/fastapi-service
git init
akmon --yes --output json --task "create FastAPI app with /health endpoint and pytest test" | tee run.json
```

### TypeScript service starter (illustrative defense supplier context)

Constraints:
- Audit need: deterministic artifact capture for external review.
- CI requirement: include at least one basic route test.

```bash
mkdir -p ~/projects/node-service && cd ~/projects/node-service
git init
akmon --yes --output json --task "create Node TypeScript service with basic route and test" | tee run.json
```

2. Verify run artifacts.

```bash
SESSION_ID="$(jq -r '.session_id' run.json)"
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
akmon verify "${SESSION_ID}"
```

## What gets recorded in evidence

- Session identifier, run status, and replay metadata hashes.
- Tool execution metrics and policy decision summary.
- Linked audit path for tamper-evident review.

## How a reviewer validates this

1. Confirm the three verification commands exit `0`.
2. Confirm `run.json` contains expected `session_id`, `status`, and `reliability_metrics`.
3. Confirm produced files and tests align with requested starter scope.

## Verification

```bash
jq '{session_id,status,files_written,reliability_metrics}' run.json
```

## Anti-patterns

- Running starter flows without storing `run.json` (loses machine-readable evidence context).
- Treating a successful run as policy-compliant without verification commands.
- Sharing artifacts externally before `audit/evidence/verify` checks pass.

## Troubleshooting

- If generation fails on provider setup, run `akmon doctor providers`.
- If verification fails, ensure `SESSION_ID` came from the same `run.json`.
