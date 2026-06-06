# Tutorial: Local-first developer flow (Ollama)

Documented for Akmon `2.2.0`.

Time estimate: 15-25 minutes  
Complexity: Beginner

## Who this is for

Developers who want a fully local Akmon workflow, with no provider API calls leaving the machine, that still produces a portable, signed, independently verifiable record of the session. This is the local-first, air-gap-friendly path: the model runs on your hardware through Ollama, and the evidence the agent produces can be handed to a reviewer and checked offline with nothing but a public key.

## What you will have at the end

- One interactive local session and one equivalent headless JSON run, using a model served by Ollama.
- Verified audit and evidence artifacts for review.
- A signed `.akmon` bundle that a third party can verify offline, including with plain `openssl`, without trusting your machine.

## Prerequisites

1. `akmon --version` prints your current build (`2.2.0` for this release).
2. `ollama` is installed and running, with the model you intend to use already pulled, so no network is needed at run time.
3. You are inside a git repository.
4. A signing key, created once with `akmon bundle keygen --out signer.pk8 --public-out signer.pub.hex`. Keep `signer.pk8` private; publish only `signer.pub.hex`.

## Steps

1. Pull a local model and verify Akmon. Pulling ahead of time means the run itself needs no network.

```bash
ollama pull qwen2.5-coder:7b
akmon --version
```

2. Start interactive mode with the local model and the `dev` policy profile.

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

5. Extract the session ID and verify the recorded artifacts.

```bash
SESSION_ID="$(jq -r '.session_id' run.json)"
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
akmon verify "${SESSION_ID}"
```

6. Export the session as a portable bundle and sign it offline.

```bash
akmon bundle export "${SESSION_ID}" --output "${SESSION_ID}.akmon"
akmon bundle sign "${SESSION_ID}.akmon" --key signer.pk8
akmon bundle verify "${SESSION_ID}.akmon" --verify-key signer.pub.hex --require-signature
```

Everything here runs locally. The model inference is Ollama on your machine, and the keygen, sign, and verify steps are offline Ed25519 operations that never contact a network. The result is a self-contained record you can move to a reviewer.

## Air-gap note

Both the inference and the trust chain work without a network. Pull the model once while connected, then the run, the signing, and the verification all work on an isolated machine. The reviewer's side is offline too: they can verify the signed bundle with `akmon bundle verify`, the standalone `agef-verify`, or plain `openssl` against `prove-openssl` artifacts. See [Verify evidence on an air-gapped machine](../use-cases/air-gapped-audit.md).

## What gets recorded in evidence

- Session metadata (session, model, and provider context, here the local Ollama model).
- Tool execution and reliability metrics.
- Replay metadata and the policy and tool-registry hashes.
- Paths to the audit and evidence artifacts for review handoff.

A reference-agent run is `full` capture, so the session is deterministically replayable and `akmon bundle verify --require-capture full` passes on its bundle.

## How a reviewer validates this

1. Confirm `akmon verify <session-id>` exits `0`.
2. Confirm `akmon audit verify` and `akmon evidence verify` both succeed.
3. Confirm `akmon bundle verify ... --verify-key signer.pub.hex --require-signature` exits `0` with a `verified` signature outcome.
4. Inspect `run.json` fields (`session_id`, `status`, `reliability_metrics`, `replay_metadata`) for expected run characteristics.

## Verification

```bash
jq '{session_id,status,reliability_metrics,replay_metadata}' run.json
```

Expected result: a JSON object with a non-empty `session_id` and `status`.

## Troubleshooting

- If Ollama is unavailable, check `ollama ps` and retry.
- If provider resolution is unexpected, run `akmon config explain-provider`.
- If the first local response is slow, warm it with `ollama run qwen2.5-coder:7b` once before rerunning.
- If `akmon bundle sign` rejects the key, regenerate it with `akmon bundle keygen`; `openssl genpkey` emits PKCS#8 v1, which the signing path rejects.

## See also

- [akmon bundle sign](../reference/sign.md)
- [akmon bundle keygen](../reference/bundle-keygen.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [Verify evidence on an air-gapped machine](../use-cases/air-gapped-audit.md)
