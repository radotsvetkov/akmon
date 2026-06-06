# Quick Start

Documented for Akmon `2.2.0`.

## Who this is for

Engineers and compliance reviewers who want one end-to-end pass through the thing that makes Akmon useful: take an agent session, seal it, sign it, and prove to a third party what happened, with nothing but `openssl` on the other end.

Akmon is a producer-agnostic evidence and verification layer. This quick start uses a session from any OpenTelemetry-instrumented agent (an OTLP/JSON GenAI trace). The same flow applies to sessions from Akmon's own bundled reference agent. The verification chain is the point; the agent that produced the trace is interchangeable.

## What you will have at the end

- A signed, portable `.akmon` bundle made from an agent trace.
- A verification that passes integrity, signature, and operator-attestation checks.
- An offline proof a stranger can verify with plain `openssl`, no Akmon install required.

## Prerequisites

1. `akmon --version` reports `2.2.0`.
2. An OpenTelemetry GenAI trace file on disk (OTLP/JSON). Akmon reads the v1.37 structured form and the older v1.36-and-earlier message-event form that most deployed agents still emit.
3. OpenSSL 3.x for the final `openssl` step. The macOS system `openssl` is LibreSSL and cannot verify Ed25519.

## The trust flow

### 1. Generate a signing key

`openssl genpkey` emits a PKCS#8 v1 key that the `ring` library rejects, so it cannot sign an Akmon bundle. Use `keygen`, which produces the PKCS#8 v2 key Akmon accepts (and sets `0600` on unix).

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub.hex
```

Keep `signer.pk8` secret. Distribute only `signer.pub.hex` (64 hex characters) to verifiers.

### 2. Import an agent trace

Turn the OpenTelemetry trace into an AGEF session. Akmon records an honest capture level: imported traces are `structural`, not a full recording, and are never dressed up as one.

```bash
akmon otel import trace.json --journal .akmon/journal
```

### 3. Export and sign the bundle

Export the session to a portable bundle, then add an offline Ed25519 signature over the session head (the `AGEF-SIG-v1` statement).

```bash
akmon bundle export <session-id> --output session.akmon
akmon bundle sign session.akmon --key signer.pk8
```

### 4. (Optional) Attest the accountable operator

Record a separately signed operator-identity claim. Verification attaches trust to the key, never to the self-asserted string; key trust is established out of band.

```bash
akmon bundle attest session.akmon \
  --key signer.pk8 \
  --operator-id you@org \
  --role approver
```

### 5. Verify

Check integrity, signature, and (if attested) operator identity in one command.

```bash
akmon bundle verify session.akmon \
  --verify-key signer.pub.hex \
  --require-signature \
  --operator-key signer.pub.hex
```

Because this session came from an OTEL import, it is `structural`. Adding `--require-capture full` here would correctly fail: that gate is reserved for full-capture sessions from Akmon's own reference agent.

### 6. Emit an offline proof

Write the exact bytes a third party needs, plus the `openssl` command to check them.

```bash
akmon bundle prove-openssl session.akmon \
  --verify-key signer.pub.hex \
  --out-dir proof
```

This writes `statement.bin`, `signature.bin`, and `pubkey.pem` into `proof/`.

### 7. Verify with plain openssl

This is the step that proves there is no lock-in. It runs on any machine with OpenSSL 3.x, with no Akmon installed.

```bash
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin -in proof/statement.bin -sigfile proof/signature.bin
```

A valid signature prints `Signature Verified Successfully` and exits `0`. Tampering with `statement.bin` makes `openssl` print a verification failure and exit non-zero.

## Verification

The flow is complete when:

- `akmon bundle verify ... --require-signature` exits `0`.
- `openssl pkeyutl -verify ...` prints `Signature Verified Successfully`.

For an auditor who has only the bundle and the public key, the standalone binary does the same integrity and signature checks without the full Akmon CLI:

```bash
agef-verify session.akmon --verify-key signer.pub.hex --require-signature
```

## Running Akmon's own reference agent

The bundled agent is the gold-fidelity producer: its sessions capture at `full` and replay deterministically. To produce one instead of importing a trace, start a session in a repository:

```bash
cd /path/to/your-repo
akmon
```

End it with `/exit`. A full-capture session can then be exported, signed, and proven exactly as above, and additionally supports `akmon bundle verify ... --require-capture full` and deterministic `akmon replay`.

## Troubleshooting

- If `openssl` cannot verify the signature, confirm you are on OpenSSL 3.x, not LibreSSL. See [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md).
- If `akmon bundle sign` rejects a key, regenerate it with `akmon bundle keygen`. An `openssl`-made key will not work.
- If `--require-capture full` fails on an imported session, that is expected. Imports are `structural`.
- If a provider call fails while running the reference agent, run `akmon doctor providers`.
