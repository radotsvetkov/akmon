# Quick start

Documented for Akmon `3.0.0`.

Seal an agent session into one signed file, then verify it the way a stranger would. Five commands.

## You need

- `akmon` and `agef-verify` on your `PATH` ([Install](./installation.md)).
- An OpenTelemetry GenAI trace exported as OTLP/JSON (`trace.json`). Any instrumented agent works. If you use the bundled reference agent instead, you can seal its session by id; see step 2.
- Optional: OpenSSL 3 for the last step. The macOS system `openssl` is LibreSSL and cannot verify Ed25519.

## 1. Make a signing key

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub
```

`signer.pk8` is your private key (created `0600` on unix). Keep it secret. `signer.pub` holds the public key as 64 hex characters; that is what you give to verifiers.

Do not use `openssl genpkey` to make the key. It writes PKCS#8 v1, which Akmon cannot sign with.

## 2. Seal the session

```bash
akmon seal trace.json --key signer.pk8 --output audit.akmon
```

`akmon seal` imports the trace, exports it as a bundle, checks its integrity, signs the session head, and verifies the result with the derived public key before it reports success. It writes:

- `audit.akmon`: the sealed bundle.
- `audit.akmon.pub`: the signer's public key in hex, the same content as `signer.pub`. It is a convenience copy for you to send to the verifier over a separate channel (step 3); the verifier should never trust a `.pub` that arrived with the bundle.

It also prints the exact commands to hand to your verifier. The input can be a session id instead of a trace file, for example a session recorded by the bundled reference agent: `akmon seal <session-id> --key signer.pk8`.

## 3. Hand it over

Send `audit.akmon` to the verifier. Send the public key over a **separate, trusted channel** (a signed email, your website, a ticket in their system). If the bundle and key travel together, whoever can swap one can swap both.

## 4. Verify as a stranger

On any machine, with only the bundle and the public key:

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature
```

Exit code `0` and a headline starting `verified bundle: integrity OK; signature verified by key_id=…` means the contents match the signed head and the key you trust signed it.

For a plain-language report an auditor can file:

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature --format markdown > report.md
```

## 5. Optional: verify with nothing but OpenSSL

Seal with `--proof-dir proof` (or run `akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof` later), then on the verifier's machine:

```bash
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin \
  -in proof/statement.bin -sigfile proof/signature.bin
```

`Signature Verified Successfully` means the key holder signed session head H. `agef-verify` is what ties H to the bundle's contents, so a careful verifier runs both.

## What you just proved, and what you did not

- Proved: the bundle has not been altered since it was sealed, and the holder of `signer.pk8` sealed it.
- Not proved: when it was sealed, who holds the key, or that the agent was right. See [What verification proves](../concepts/verifying-evidence.md) and [Trusted timestamps](../guides/trusted-timestamps.md).

The capture level tells the verifier how much of the session was recorded. A trace is `full` only when every content slot (each request, response, tool input and output) holds real content; a trace with some content is `partial`, and a metadata-only trace is `structural`. See [Capture levels](../concepts/capture-levels.md).

## Next steps

- [Seal an OpenTelemetry trace from any agent](../guides/seal-otel-trace.md)
- [Hand evidence to an auditor](../guides/hand-to-auditor.md)
- [Record who approved a session](../use-cases/operator-sign-off.md)
- [`akmon seal` reference](../reference/seal.md)

## Troubleshooting

- `akmon seal` refuses to overwrite `audit.akmon`: pass `--force`, or choose another `--output`.
- The trace holds several traces: pass `--trace-id <HEX>`.
- `openssl` cannot verify: you are probably on LibreSSL. Install OpenSSL 3.
- `--require-capture full` fails: the trace did not include message content. That is the capture level working as intended.
