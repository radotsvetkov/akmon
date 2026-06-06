# Verify evidence on an air-gapped machine

Documented for Akmon `2.2.0`.

## Who this is for

An auditor, regulator, or counterparty who received a `.akmon` bundle and a public key and needs to check it independently. You may be on an air-gapped machine, you may not run Akmon, and you do not need to trust whoever produced the bundle. This page shows three ways to verify, in increasing order of independence, and how to read the outcome honestly.

## What you should have

- The `.akmon` bundle file.
- The signer's public key as 64 hex characters, and, if accountability matters, the operator's public key, both obtained through a channel you trust. Key trust is out of band; Akmon does not distribute keys.

## Three ways to verify

Each method checks the same record. They differ in how much of the producer's tooling you must trust.

### 1. Akmon's own verifier

If you have Akmon installed, this runs every stage of the chain in one command.

```bash
akmon bundle verify /path/to/session.akmon \
  --verify-key signer.pub.hex \
  --require-signature \
  --operator-key operator.pub.hex \
  --require-operator \
  --require-capture full
```

Drop the require flags you do not need. Without `--require-signature`, an unsigned or unknown-key bundle is reported, not failed; with it, missing provenance is a hard failure. The same applies to the operator and capture requirements.

### 2. The standalone verifier

`agef-verify` is a minimal binary that performs the bundle integrity, signature, operator, and capture checks without the full Akmon agent. Install it on its own (Homebrew tap or release binary). Use this when you want to verify without bringing in the whole agent.

```bash
agef-verify /path/to/session.akmon \
  --verify-key signer.pub.hex \
  --require-signature
```

It accepts `--operator-key`, `--require-operator`, and `--require-operator-key` with the same semantics as `akmon bundle verify`.

### 3. Plain openssl, no Akmon at all

This is the strongest independence: verify the Ed25519 signature with stock `openssl` and nothing else. Someone with Akmon (the producer, or you on a connected machine) first emits the proof artifacts; the artifacts then travel to the air-gapped machine, where only `openssl` is needed.

```bash
akmon bundle prove-openssl /path/to/session.akmon \
  --verify-key signer.pub.hex \
  --operator-key operator.pub.hex \
  --out-dir ./proof
```

That writes `statement.bin` (the exact signed bytes), `signature.bin` (the raw 64-byte signature), and `pubkey.pem` (the signer's public key in PEM form), plus the operator equivalents (`operator_statement.bin`, `operator_signature.bin`, `operator_pubkey.pem`) when `--operator-key` is given. Then verify the head signature:

```bash
openssl pkeyutl -verify -pubin -inkey ./proof/pubkey.pem -rawin -in ./proof/statement.bin -sigfile ./proof/signature.bin
```

A valid signature prints a success line and exits `0`. To check the operator attestation, run the same command against `operator_pubkey.pem`, `operator_statement.bin`, and `operator_signature.bin`. The exact commands are also printed by `prove-openssl`.

OpenSSL 3.x note: you need OpenSSL 3.x for this step. The macOS system `openssl` is LibreSSL and cannot verify Ed25519; it lacks `-rawin` and cannot load the key. Use an OpenSSL 3.x build.

## How to read the outcome

A verification result is not a single yes or no. Read each signal for what it actually says.

- Verified. Integrity holds, and any provenance or accountability you required checked out against a key you supplied. You can conclude the record was not altered and that the holders of the named keys sealed and, if attested, claimed it. You still cannot conclude the agent was correct or that any person holds the key; that is out of band.
- Invalid (hard fail). Integrity failed, or a check you required did not pass. Do not rely on the record. If integrity failed, the bytes were altered or corrupted and nothing downstream is trustworthy. Stop here.
- Unverified, no key (`unverified_no_key`). The bundle carries a signature or attestation, but you did not supply a matching trusted key, so provenance is not yet established. This is not a failure on its own. Obtain the right key out of band and re-run, or require it explicitly if its absence should fail.
- Unattributed. No operator attestation is present, or none verifies against a key you trust. The record may still have integrity and a valid head signature, but no operator key has claimed accountability. If you need a named, key-backed claim, treat this as insufficient and require it.
- `capture_level: structural` versus `full`. A structural record (an OpenTelemetry import) captures the shape of the session, not a complete recording, and cannot be replayed deterministically. A full record (Akmon's own reference agent) is a complete, replayable recording. If your evidence needs a full recording, require it with `--require-capture full`; a structural import will fail that check, which is the honest result. Do not read a structural import as a full recording.

## A note on stripped trust metadata

Signatures and operator attestations are additive manifest fields. An intermediary who controls the file can remove them, and the remaining record still verifies for integrity. Cryptography cannot prove that something absent was once present. If provenance or accountability matters, do not rely on a signature merely being present in a bundle you happened to receive. Require it with the flags above and supply the trusted keys, so a stripped or unknown-key bundle fails rather than passes quietly.

## See also

- [Verifying evidence (for auditors)](../concepts/verifying-evidence.md)
- [Trust and threat model](../concepts/trust-model.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [agef-verify](../reference/agef-verify.md)
- [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md)
- [Record who approved an AI change](./operator-sign-off.md)
