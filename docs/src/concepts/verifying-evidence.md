# What verification proves

Documented for Akmon `3.0.0`.

This page is for the person who received a `.akmon` bundle and a public key and has to decide what it is worth. You may be air-gapped, you may not run Akmon, and you do not need to trust whoever produced the bundle. For the underlying guarantees, see [Trust model](./trust-model.md).

## What you should have

- The `.akmon` bundle.
- The signer's public key (64 hex characters in a file), obtained through a channel you trust, separate from the bundle. If accountability matters, the operator's public key too. Akmon does not distribute or vouch for keys.

## Three ways to verify, increasing independence

### 1. `agef-verify` (recommended)

A small standalone binary with no agent, journal, or network code.

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature
agef-verify audit.akmon --verify-key signer.pub --require-signature --format markdown > report.md
```

Add `--operator-key operator.pub --require-operator` to require a key-backed operator claim, and `--require-capture full` to require a complete recording.

### 2. `akmon bundle verify`

The same checks and flags, built into the full `akmon` CLI. See [akmon bundle verify](../reference/bundle-verify.md).

### 3. Stock OpenSSL 3, no Akmon at all

Someone with Akmon (the producer, or you) writes the proof artifacts, with `akmon seal … --proof-dir proof` or:

```bash
akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof
```

Then anyone checks the signature:

```bash
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin \
  -in proof/statement.bin -sigfile proof/signature.bin
```

This proves the key holder signed session head H. It does not by itself tie H to the bundle's contents; that is what the hash-chain check in `agef-verify` does. Use OpenSSL 3: the macOS system `openssl` is LibreSSL and cannot verify Ed25519.

## What a passing verification proves

- **Integrity.** Every event and object in the bundle is exactly what the session head commits to. Nothing was changed, added, or removed after the head was computed. (On its own this only shows internal consistency: anyone can build a consistent bundle. Authorship comes from the next point.)
- **Signature.** With `--require-signature` and a key you trust: the holder of that private key signed this exact session head, and therefore these exact contents.
- **Operator attestation.** With `--require-operator` and an operator key you trust: the holder of that key signed the operator identity fields together with the session head.
- **Capture level.** How much of the session was recorded (`full`, `partial`, or `structural`). The level is inside the signed record, so it cannot be upgraded after sealing.

## What it does not prove

- **When.** Event timestamps come from the producer's clock, and the manifest's `created_at` and `ended_at` are not signed at all. No trusted time is part of the signature yet. If time matters, use an RFC 3161 timestamp alongside the bundle: see [Trusted timestamps](../guides/trusted-timestamps.md).
- **Who holds the key.** Binding a public key to a person or organization happens outside Akmon (a key ceremony, a signed roster, a PKI).
- **Who the operator is.** `operator_id`, `role`, and `org` are self-asserted text. Only the key is verified.
- **That the agent was right,** or that the session complied with any rule or obligation.
- **That nothing was left out.** A bundle records the session the producer sealed. It cannot prove that a different, unsealed session did not also happen.
- **For imported traces, that the trace was complete.** Akmon records what the OpenTelemetry trace contained. If the instrumentation dropped spans or omitted content, the capture level shows the omitted content, but missing spans cannot be detected.

## Reading the headline

| Headline | What to conclude |
| --- | --- |
| `verified bundle: integrity OK; signature verified by key_id=<id>` | Intact and signed by a key you trust. |
| `…; signature present but NOT verified (…integrity only)` | Signed, but not by any key you supplied. Get the right key out of band, or treat as unsigned. |
| `…; UNSIGNED (integrity only)` | No signature. Integrity alone proves little. |
| `bundle verification FAILED` | Do not rely on it. The violations list says what broke. |

In JSON, read `passed`, `integrity_ok`, and `signature_verified` together. Without `--require-signature`, `passed: true` means integrity only.

Operator outcomes: `verified` (against a key you supplied), `unverified_no_key` (attested, but you supplied no matching key; not a failure on its own), or unattributed (no attestation).

## Stripped signatures

Signatures and attestations are additive fields. Someone who controls the file can remove them, and the rest still verifies for integrity. Cryptography cannot prove that something absent was once there. If provenance matters, always pass `--require-signature` (and `--require-operator`) with the trusted keys, so a stripped bundle fails instead of quietly passing.

## See also

- [Trust model](./trust-model.md)
- [Capture levels](./capture-levels.md)
- [Hand evidence to an auditor](../guides/hand-to-auditor.md)
- [agef-verify](../reference/agef-verify.md)
- [Compliance crosswalk](./compliance.md)
