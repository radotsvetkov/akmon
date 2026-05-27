# Tutorial: Local-first developer flow (Ollama)

Documented for Akmon `2.1.0`.

Time estimate: 15-20 minutes  
Complexity: Beginner

## Who this is for

Developers who want a fully local Akmon workflow with verifiable session evidence.

## What you will have at the end

- One interactive local session.
- One equivalent headless JSON run.
- Verified audit/evidence artifacts for review.

## Prerequisites

1. `akmon --version` prints `2.1.0` (or your current build).
2. `ollama` is installed and running.
3. You are inside a git repository.

## Steps

1. Pull a local model and verify Akmon.

```bash
ollama pull qwen2.5-coder:7b
akmon --version
```

2. Start interactive mode with a local model and dev policy profile.

```bash
cd /path/to/your-repo
akmon --model qwen2.5-coder:7b --policy-profile dev chat
```

3. Run one controlled implementation request.

```text
add validation to the registration handler and update tests
```

Expected result: Akmon asks for approvals before write actions.

4. Run an equivalent headless task for machine-readable artifact output.

```bash
akmon --model qwen2.5-coder:7b --yes --output json \
  --task "add validation to the registration handler and update tests" \
  | tee run.json
```

5. Extract the session ID and verify recorded artifacts.

```bash
SESSION_ID="$(jq -r '.session_id' run.json)"
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
akmon verify "${SESSION_ID}"
```

## What gets recorded in evidence

- Session metadata (session/model/provider context).
- Tool execution and reliability metrics.
- Replay metadata and policy/tool registry hashes.
- Paths to audit/evidence artifacts for review handoff.

## How a reviewer validates this

1. Confirm `akmon verify <session-id>` exits `0`.
2. Confirm `akmon audit verify` and `akmon evidence verify` both succeed.
3. Inspect `run.json` fields (`session_id`, `status`, `reliability_metrics`, `replay_metadata`) for expected run characteristics.

## Verification

```bash
jq '{session_id,status,reliability_metrics,replay_metadata}' run.json
```

Expected result: JSON object includes non-empty `session_id` and `status`.

## Troubleshooting

- If Ollama is unavailable, check `ollama ps` and retry.
- If provider resolution is unexpected, run `akmon config explain-provider`.
- If first local response is slow, warm with `ollama run qwen2.5-coder:7b` once before rerunning.
