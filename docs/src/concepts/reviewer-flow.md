# Regulated Reviewer Flow

Documented for Akmon `3.0.0`.

## Who this is for

Reviewers, tech leads, compliance engineers, and external auditors validating AI-assisted sessions. This page is the checklist from a received bundle to a verification-ready, signature-backed handoff.

It covers both kinds of session Akmon handles: sessions imported from another agent's OpenTelemetry trace (capture level `full`, `partial`, or `structural`, depending on the trace) and sessions produced by the bundled reference agent (`full`). The verification chain is the same; only the capture level and the availability of replay differ.

## What you will have at the end

- A repeatable checklist that ends in an independently verifiable signature.
- A clear decision on whether a bundle is review-ready and safe to distribute.

## Prerequisites

1. A `.akmon` bundle, or a completed Akmon run with a session ID.
2. The signer's public key (64 hex characters) for any signature you must check, established through your own out-of-band trust process.
3. OpenSSL 3.x if you intend to reproduce the proof with plain `openssl`.

## Steps

### 1. Verify bundle integrity, signature, and operator identity

For a received bundle, this is the primary check. It validates the hash-linked event chain, the manifest head, the offline Ed25519 signature over that head, and any operator attestation.

```bash
akmon bundle verify session.akmon \
  --verify-key signer.pub \
  --require-signature \
  --operator-key operator.pub
```

"Verified" attaches to the key, not to the self-asserted operator name. A bundle that carries an attestation but is verified without a trusted key reports `unverified_no_key`, which is informational, not a failure.

An auditor who does not run the full Akmon CLI can use the standalone binary for the same integrity and signature checks:

```bash
agef-verify session.akmon --verify-key signer.pub --require-signature
```

### 2. Enforce capture level when the use case demands a full recording

```bash
akmon bundle verify session.akmon --verify-key signer.pub --require-capture full
```

This fails on `partial` and `structural` sessions, which is the honest result. Decide up front which categories of change require full content and gate accordingly.

### 3. Reproduce the proof offline with openssl (when zero-trust verification is required)

If a counterparty does not trust your tools, hand them the proof artifacts and the exact command.

```bash
akmon bundle prove-openssl session.akmon --verify-key signer.pub --out-dir proof
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin -in proof/statement.bin -sigfile proof/signature.bin
```

A valid signature prints `Signature Verified Successfully`. This needs only OpenSSL 3.x, no Akmon install.

### 4. For reference-agent sessions in a local journal, check the journal too

When the session lives in a local journal rather than arriving as a bundle:

```bash
SESSION_ID="<session-uuid>"
akmon verify "${SESSION_ID}"
```

### 5. Replay for behavioral divergence (full capture only)

```bash
akmon replay "${SESSION_ID}" --format json | tee replay.json
```

Replay applies to sessions from the bundled reference agent. OpenTelemetry imports are refused for replay at any capture level, because an imported trace is an evidence record, not an execution.

### 6. Seal, and (if needed) redact for archive or external review

```bash
akmon seal "${SESSION_ID}" --key signer.pk8 --output "${SESSION_ID}.akmon"
```

If sensitive content must be removed, create a derivative redacted bundle and re-verify it before distribution:

```bash
akmon redact "${SESSION_ID}" \
  --output "${SESSION_ID}-sanitized.akmon" \
  --object <object-hash> \
  --reason "compliance redaction"
akmon bundle sign "${SESSION_ID}-sanitized.akmon" --key signer.pk8
```

A redacted bundle is a new derivative with its own head; sign it again and keep the original sealed bundle under retention.

## Verification

A handoff is review-ready when:

- `akmon bundle verify ... --require-signature` exits `0`, and the signature checks against a key you trust out of band,
- the capture level matches what the use case requires (`--require-capture full` where mandated),
- for zero-trust handoff, `openssl pkeyutl -verify ...` prints `Signature Verified Successfully`,
- for full-capture sessions, replay is pass or divergences are explicitly accepted,
- any redacted bundle still passes `akmon bundle verify` before it leaves your control.

## Troubleshooting

- If `bundle verify` fails, stop the review and inspect the violation category before proceeding.
- If `--require-capture full` fails on an imported session, the trace did not carry all message content. Treat it as `partial` or `structural` evidence.
- If `openssl` cannot verify the signature, confirm OpenSSL 3.x rather than LibreSSL.
- If `replay` diverges, treat it as a change-detection signal and triage expected versus unexpected drift.
- If bundle verification fails, do not distribute the bundle externally.
