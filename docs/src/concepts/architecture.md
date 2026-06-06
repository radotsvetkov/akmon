# How Akmon works

Documented for Akmon `2.2.0`.

## Who this is for

Engineers and architects who want a clear picture of how Akmon records an AI agent session, how the
trust layer attaches without changing that record, and how a third party verifies it offline. This
page describes the model. For the security boundaries, read
[Trust and threat model](./trust-model.md).

## The producer-agnostic model

Akmon is an evidence and verification layer that sits on top of whatever agent you run. There are two
ways a session becomes an Akmon record, and they differ in fidelity. Akmon labels the difference
honestly with a `capture_level` so a reader is never misled.

| Producer | How it enters Akmon | `capture_level` | Replayable | Notes |
| --- | --- | --- | --- | --- |
| Akmon bundled reference agent | Run directly under Akmon | `full` | Yes, deterministically | The gold-fidelity reference producer. |
| Any agent via OpenTelemetry | `akmon otel import <trace.json>` | `structural` | No (`replay` refuses it) | Captures the shape of a GenAI trace, not a complete recording. |

The bundled coding agent is the reference producer, not the headline. The point of Akmon is that the
record and its verification are the same regardless of who produced the session. A regulator or
auditor verifies a bundle the same way whether it came from Akmon's own agent or from an imported
trace. What changes is only how much the record can claim, and that claim is carried in
`capture_level`.

`akmon otel import` reads the OTLP/JSON OpenTelemetry GenAI form. It accepts both the v1.37 structured
form and the legacy v1.36-and-earlier message-event form, and records `capture_level: structural` in
both cases.

## The AGEF substrate

Every session, from either producer, lands on the same substrate: the AGEF format.

- **Events.** The session is a sequence of events. Each event commits to its predecessor, forming a
  hash-linked chain.
- **Object store.** Events and their payloads are stored as content-addressed objects, each named by
  its SHA-256 digest. The name is a commitment to the bytes.
- **Head.** The head is the hash of the terminal event. Because the chain is hash-linked, the head
  commits to the entire directed acyclic graph of objects reachable from it. One value pins the whole
  record.
- **Portable bundle.** A session exports to a single `tar.zst` `.akmon` file containing the manifest,
  the event stream, and the referenced objects. The bundle is self-contained and moves between
  machines, organizations, and air-gapped environments without any Akmon service.
- **Manifest.** The manifest describes the bundle and carries the head plus the optional trust-layer
  fields described next.

## The additive trust layers

Provenance and accountability are layered on top of the AGEF substrate without touching it. This is
the important property: adding or checking trust never changes the event chain or the head.

- **`manifest.signatures[]` (AGEF v0.1.2).** Zero or more Ed25519 detached signatures over the
  domain-separated `AGEF-SIG-v1` statement (which commits to the head). Added by
  [`akmon bundle sign`](../reference/sign.md). The signing key is created by
  [`akmon bundle keygen`](../reference/bundle-keygen.md), which produces an Ed25519 PKCS#8 v2 key that
  the `ring` crate accepts.
- **`manifest.operator_attestations[]` (AGEF v0.1.3).** Zero or more separately-signed
  `AGEF-OPERATOR-v1` operator-identity claims, each binding the head and session id to self-asserted
  identity fields. Added by [`akmon bundle attest`](../reference/bundle-attest.md).

Both fields are optional. A bundle with neither still verifies for integrity. Because they are
additive metadata, an intermediary can strip them, so a verifier that requires them must say so with
the require flags and supply trusted keys. See [Trust and threat model](./trust-model.md).

Akmon implements AGEF v0.1.3. Ed25519 is provided by the `ring` crate.

## The verification chain

Verification runs as a sequence of independent stages. A later stage does not compensate for an
earlier failure, and each stage trusts only the keys and requirements you supply.

1. **Integrity.** Re-hash every object and rewalk the event chain to reproduce the head. This is the
   foundation. If it fails, stop.
2. **Head signature.** Check `manifest.signatures[]` against a trusted signer public key. With
   `--require-signature` a missing or unverifiable signature is a hard failure.
3. **Operator attestation.** Check `manifest.operator_attestations[]` against a trusted operator
   public key. With `--require-operator` or `--require-operator-key` a missing or unverifiable
   attestation is a hard failure. "Verified" attaches to the key, not the self-asserted name.
4. **Capture level.** With `--require-capture full`, a structural import fails. This keeps a
   structural record from being treated as a full, replayable one.

These stages are exposed by [`akmon bundle verify`](../reference/bundle-verify.md).

## The standalone verifier and the openssl path

Verification does not require a full Akmon install, and the strongest form does not require Akmon at
all.

- **`agef-verify`.** A standalone binary that performs the bundle integrity, signature, operator, and
  capture checks without the full Akmon CLI. Install it on its own (Homebrew tap or release binary).
  See [agef-verify](../reference/agef-verify.md).
- **Plain `openssl`.** [`akmon bundle prove-openssl`](../reference/bundle-prove-openssl.md) writes the
  exact signed statement bytes, the raw signature, and the public key in PEM form, plus the precise
  `openssl pkeyutl -verify` command. A third party then verifies the Ed25519 signature with stock
  `openssl` alone: no Akmon binary, no cloud, no need to trust the producer. This step needs OpenSSL
  3.x; the macOS LibreSSL `openssl` cannot verify Ed25519.

That openssl path is the point of the whole design. The record is portable and content-addressed, the
signature is a standard detached Ed25519 signature over a canonical statement, and the only thing a
verifier needs is the public key they already trust.

## See also

- [Trust and threat model](./trust-model.md)
- [Verifying evidence (for auditors)](./verifying-evidence.md)
- [Compliance and evidence](./compliance.md)
- [Glossary](./glossary.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [agef-verify](../reference/agef-verify.md)
- [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md)
