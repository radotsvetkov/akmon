# Tutorial: Third-party OTEL trace to offline openssl proof

Documented for Akmon `2.1.0`.

Time estimate: 15-20 minutes  
Complexity: Intermediate

## Who this is for

Teams whose agents are **already** instrumented with a third-party OpenTelemetry GenAI
instrumentation, who want a signed, **standalone-verifiable** audit record of a session, and
auditors who must check that record **with stock `openssl` alone**: no Akmon binary, no cloud,
no vendor lock-in.

This is the headline trust loop, end to end, on what real agents emit **today**: a real-framework
OTLP trace becomes an AGEF bundle that a counterparty verifies offline. It is the concrete answer
to the gaps competitors leave: HMAC-only or unsigned manifests, no standalone verifier,
cloud-locked verification, and "cannot replay."

## What you will have at the end

- An AGEF bundle built from a third-party OpenTelemetry GenAI trace.
- An Ed25519 **signature over the session head**, verifiable by anyone who trusts the public key.
- Three artifacts (`statement.bin`, `signature.bin`, `pubkey.pem`) that a third party verifies
  with **OpenSSL 3.x** and nothing else.

## The fixture

This walkthrough uses the checked-in fixture
`crates/akmon-cli/tests/fixtures/openai_v2_weather_legacy.otlp.json`. It is a **representative,
illustrative** OTLP/JSON trace that models the **default** emission of the
`opentelemetry-instrumentation-openai-v2` Python instrumentation, in the **legacy (`<= v1.36`)
message-event form** (`gen_ai.system.message` / `gen_ai.user.message` / `gen_ai.choice` span
events). It is **hand-authored to match that documented shape**, contains **no real user data or
PII**, and, because that instrumentation does not capture message content unless
`OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` is enabled (default **off**), carries no
message bodies. See `crates/akmon-cli/tests/fixtures/README.md` for the full provenance note.

## Prerequisites

1. `akmon` installed and on `PATH`.
2. **OpenSSL 3.x** for the final verification step. Stock LibreSSL (the macOS `/usr/bin/openssl`)
   cannot verify Ed25519: it lacks `-rawin` and cannot load Ed25519 keys.

## Steps

1. Import the third-party OTEL trace into a fresh AGEF session.

```bash
akmon otel import crates/akmon-cli/tests/fixtures/openai_v2_weather_legacy.otlp.json \
  --journal ./journal --format json
```

The JSON report records `capture_level`, the provider/tool counts, and the new `session_id`:

```json
{
  "capture_level": "structural",
  "provider_calls": 1,
  "tool_calls": 1,
  "turns_emitted": 0,
  "turns_suppressed_no_content": 1,
  "semconv_version": "1.37.0",
  "session_id": "de52d29b-e7ee-4f53-8526-3c479d4f8c37"
}
```

2. Export the session as an AGEF bundle.

```bash
akmon bundle export <session-id> --journal ./journal --output ./audit.akmon
```

3. Sign the bundle's session head with an Ed25519 key and publish the public key (hex).

```bash
akmon bundle sign ./audit.akmon --key signer.pk8 --format json
```

4. Verify integrity and the signature, requiring a signature to be present.

```bash
akmon bundle verify ./audit.akmon --verify-key signer.pub.hex --require-signature --format json
```

5. Emit the standalone verification artifacts.

```bash
akmon bundle prove-openssl ./audit.akmon --verify-key signer.pub.hex --out-dir ./proof
```

6. Verify offline with **OpenSSL 3.x** alone (this command is also printed by the step above).

```bash
openssl pkeyutl -verify -pubin -inkey ./proof/pubkey.pem -rawin -in ./proof/statement.bin -sigfile ./proof/signature.bin
```

A valid signature prints `Signature Verified Successfully` and exits `0`. Tampering with
`statement.bin` (or using the wrong signature) makes `openssl` print `Signature Verification
Failure` and exit non-zero.

## Optional: bind an operator identity

The head signature proves the bundle's integrity is authentic, but says nothing about **who**
operated the session. To attach a named operator (and have it verify offline too), generate an
operator key, attest, and pass `--operator-key` to verify and to `prove-openssl`.

```bash
akmon bundle keygen --out operator.pk8 --public-out operator.pub.hex
akmon bundle attest ./audit.akmon --key operator.pk8 --operator-id ops@example.com --role approver
akmon bundle verify ./audit.akmon --verify-key signer.pub.hex --require-signature \
  --operator-key operator.pub.hex --require-operator --format json
akmon bundle prove-openssl ./audit.akmon --verify-key signer.pub.hex \
  --operator-key operator.pub.hex --out-dir ./proof
```

The last step emits three more files alongside the head-signature artifacts:
`operator_statement.bin`, `operator_signature.bin`, `operator_pubkey.pem`, and a third party
verifies the operator attestation with OpenSSL 3.x alone:

```bash
openssl pkeyutl -verify -pubin -inkey ./proof/operator_pubkey.pem -rawin -in ./proof/operator_statement.bin -sigfile ./proof/operator_signature.bin
```

**Trust the key, not the name.** Verification proves only that the holder of `operator.pub.hex`
signed the `operator_id`/`role` claims. It does not prove the person is who the name says. A
verifier decides which operator key they trust **out-of-band** (a directory, a roster, a key
ceremony); only then does the self-asserted name carry weight. See
[akmon bundle attest](../reference/bundle-attest.md).

## Honesty: this is STRUCTURAL capture, not full replay

The source instrumentation did not capture message content (the content-off default), so Akmon
imports the trace as **`capture_level=structural`**: metadata only. This is surfaced, not hidden:

- The import report and `akmon bundle verify --format json` both report the level as `structural`
  (under `/capture/level`).
- The full-capture gate **correctly fails** on this bundle:

```bash
akmon bundle verify ./audit.akmon --require-capture full
```

This exits `1`: a metadata-only OTEL import must never read as VERIFIED-full. The integrity and
signature still verify (the **evidence** is intact and authentic); what is *absent* is the
verbatim message content, so **no byte-level or full replay is implied** from imported telemetry.

A note on versions: the fixture's **source form** is the legacy `<= v1.36` message-event
convention, while Akmon records `source_semconv 1.37.0` in the signed session config for all
imports regardless of the source form (a hardcoded constant). The recorded value is therefore not
a faithful descriptor of the source form; this is documented in the fixture README and is cosmetic.

## How a reviewer validates this

1. Confirm `akmon otel import` exits `0` and reports `capture_level=structural`.
2. Confirm `akmon bundle verify --verify-key --require-signature` exits `0` with a `verified`
   signature outcome and `/capture/level` equal to `structural`.
3. Confirm `akmon bundle verify --require-capture full` exits `1`.
4. Confirm the OpenSSL 3.x command exits `0` for the emitted artifacts and non-zero when
   `statement.bin` is tampered.

## Verified by an automated test

This entire chain (import, export, sign, verify, `--require-capture full` failure,
`prove-openssl`, and the real `openssl` positive/tamper-negative legs) is asserted as ONE flow by
`t_e2e_otel_legacy_trace_to_openssl_proof` in
`crates/akmon-cli/tests/e2e_otel_to_openssl_integration.rs`, against the same fixture. A companion
test, `t_e2e_otel_proof_artifacts_byte_identical`, locks the emitted artifacts byte-for-byte
without requiring openssl, so the proof holds even where the openssl leg skips. Doc and test cannot
drift.

## Troubleshooting

- If the openssl step reports `unable to load` or usage text, you are likely on LibreSSL. Use an
  OpenSSL 3.x build (the verification command needs `-rawin` and Ed25519 support).
- If `akmon bundle sign` rejects the key, generate a PKCS#8 **v2** Ed25519 key; some tools emit
  PKCS#8 v1, which the signing path rejects.

## See also

- [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [akmon sign](../reference/sign.md)
