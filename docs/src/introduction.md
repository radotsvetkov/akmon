# Introduction

Akmon does one job: it **seals an AI agent session into one signed file that a stranger can verify offline**.

You hand over a single `.akmon` bundle and a public key. The person on the other end, an auditor, a regulator, a customer, or your own incident team, checks it with the small `agef-verify` binary or with stock OpenSSL 3. They do not need an Akmon install, a cloud account, or any trust in you.

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub
akmon seal trace.json --key signer.pk8 --output audit.akmon
# send audit.akmon, and signer.pub over a separate trusted channel

agef-verify audit.akmon --verify-key signer.pub --require-signature
```

That is the whole product. The [Quick start](./getting-started/quickstart.md) walks through it in five commands.

## The problem

When an AI agent changes something, you may have to prove later what it actually did. The person asking may not trust you and may not run your tools, and they may ask months after the fact.

Most agent telemetry cannot answer that. It lives in a vendor console or in mutable, unsigned spans. Anyone with write access can edit it, and nobody outside your organization can check it.

Akmon turns a session into evidence that survives that test:

- **Tamper-evident.** Every event and payload is content-addressed and hash-linked. One value, the session head, commits to the whole record. Changing, adding, or removing anything breaks verification.
- **Signed.** An Ed25519 signature over the head says which key sealed the record. A separate operator attestation can record the person who approved it.
- **Offline-verifiable.** The bundle is one portable file. Verification needs only the file, the public key, and `agef-verify` or OpenSSL 3.
- **Honest about what it captured.** Every bundle carries a capture level (`full`, `partial`, or `structural`) inside the signed record, so a metadata-only import can never pass for a full recording.

## Where sessions come from

Akmon is producer-agnostic.

- **Any agent, through OpenTelemetry.** If your agent emits OpenTelemetry GenAI traces, export one as OTLP/JSON and run `akmon seal trace.json`. See [Seal an OpenTelemetry trace](./guides/seal-otel-trace.md).
- **The bundled reference agent.** Akmon ships a coding agent that records sessions at full capture. It is the reference producer, and the only one whose sessions `akmon replay` can re-run deterministically. See [Bundled reference agent](./usage/interactive.md).

## What a verified bundle proves, and what it does not

A verified bundle proves that the contents are exactly what the session head commits to, and that the holder of a trusted key signed that head.

It does not prove when the session happened (no trusted timestamp is signed yet; see [Trusted timestamps](./guides/trusted-timestamps.md)), who a key belongs to, or that the agent behaved correctly. Read [What verification proves](./concepts/verifying-evidence.md) before you rely on a bundle.

## Regulation

Akmon helps you produce evidence. It is not a certification and does not make anything compliant. The [Compliance crosswalk](./concepts/compliance.md) maps Akmon evidence to the EU AI Act (as amended by the Digital Omnibus on AI), DORA, NIS2, NIST AI RMF, and SOC 2, including what each obligation needs that Akmon does not cover.

## Where to go next

- [Quick start](./getting-started/quickstart.md): seal and verify a session.
- [Install](./getting-started/installation.md): Homebrew, prebuilt binaries, or source.
- [Trust model](./concepts/trust-model.md): what you have to trust, and what you do not.
- [Hand evidence to an auditor](./guides/hand-to-auditor.md): what to send and what to tell them.
- [`akmon seal` reference](./reference/seal.md).
