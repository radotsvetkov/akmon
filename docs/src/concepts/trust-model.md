# Trust and threat model

Documented for Akmon `2.2.0`.

## Who this is for

Security engineers, auditors, and reviewers who need to understand exactly what a verified Akmon
record proves, what it does not prove, and the cryptographic and threat assumptions behind it. Read
this before you rely on an Akmon bundle as evidence.

## What a verified record proves

Akmon turns an AI agent session into a portable, content-addressed record. Verification answers a
small, precise set of questions. Treat each one independently.

- **Integrity.** The session record is internally consistent and nothing was altered after the fact.
  Every object is content-addressed by SHA-256, the events form a hash-linked chain, and the head is
  the hash of the terminal event. Re-hashing the objects and rewalking the chain reproduces the head.
  If any byte changed, the head changes and verification fails.
- **Which key sealed it.** When the bundle carries an Ed25519 signature, a successful check with a
  public key you supply proves that the holder of the matching private key signed this exact head.
  It proves the key, not the person.
- **Which operator key claims accountability.** When the bundle carries an operator attestation, a
  successful check with an operator public key you supply proves that the holder of that key signed a
  statement binding the session head to a set of self-asserted identity fields. Again, this proves
  the key, not the named person.

## What a verified record does NOT prove

These boundaries are deliberate. State them plainly to anyone who consumes the evidence.

- **It does not make the agent safe or correct.** Akmon records what happened. It does not judge
  whether the agent's actions were good, safe, authorized, or free of error. A faithfully recorded,
  cryptographically sound bundle can still describe a bad session.
- **It does not certify compliance.** Akmon produces evidence that supports record-keeping
  obligations. It is not a certification and does not guarantee that any regulation or control was
  satisfied. See [Compliance and evidence](./compliance.md).
- **A signature proves which key signed, not who holds it.** Binding a key to a person or
  organization is out of band. Akmon has no view into who controls a private key. If you want a name,
  you must establish key ownership through your own channels.
- **An imported trace is structural, not a full recording.** A session brought in through
  OpenTelemetry records `capture_level: structural`. It captures the shape of the session, not a
  complete, replayable recording. Only Akmon's own bundled reference agent produces
  `capture_level: full`, which replays deterministically. Do not read a structural import as a full
  recording.

## Cryptographic design

- **Content addressing.** Every object (events, payloads, referenced blobs) is named by its SHA-256
  digest. The name is a commitment to the bytes.
- **Hash-linked event chain.** Each event commits to its predecessor, forming a chain. The head is
  the hash of the terminal event, so the head commits to the entire directed acyclic graph of
  objects reachable from it. One head value pins the whole record.
- **Head signature.** An Ed25519 detached signature is computed over a domain-separated `AGEF-SIG-v1`
  statement (a canonical ASCII block: version tag, AGEF version, hash algorithm, session id, and
  head, each on its own line). Domain separation means the signed bytes cannot be mistaken for any
  other kind of signed message. Signing is offline and requires no network.
- **Operator attestation.** A separate, additive Ed25519 signature is computed over an
  `AGEF-OPERATOR-v1` statement, which binds the session head and session id to the operator's
  self-asserted identity fields. It is independent of the head signature and never alters the event
  chain.

Signatures live in `manifest.signatures[]` (AGEF v0.1.2) and operator attestations live in
`manifest.operator_attestations[]` (AGEF v0.1.3). Akmon implements AGEF v0.1.3. Ed25519 is provided
by the `ring` crate, which is already in the tree.

## Threat scenarios and how they are handled

- **Tampering with any event or object.** Changing any byte changes that object's SHA-256 name,
  which breaks the chain link that referenced it, which changes the terminal event hash, which
  changes the head. Any head signature computed over the old head no longer verifies. Tampering is
  detected, not silently accepted.
- **Transplanting an attestation onto another session.** The operator attestation binds the head and
  the session id, so an attestation lifted from one bundle does not verify against a different
  session. Accountability claims cannot be moved between records.
- **Stripping signatures or attestations (honest limitation).** Signatures and operator attestations
  are additive manifest metadata. An intermediary who controls the file can remove them and the
  remaining record still verifies for integrity. Cryptography cannot prove the absence of something
  that was removed. The mitigation is policy, not math: a verifier that cares must pass
  `--require-signature` and/or `--require-operator` (or `--require-operator-key`) and supply the
  trusted keys. Presence cannot be proven cryptographically. It can only be required by the verifier.
- **Substituting a different signing key.** Verification only trusts keys you supply. A bundle signed
  by an unknown key reports `unverified_no_key`, not `verified`. You decide which keys are
  authoritative.

## Key trust model

Key trust is out of band, by design.

- There is no PKI, no DID, no certificate authority.
- There is no transparency log.
- There is no key distribution or discovery mechanism in Akmon.

You obtain the signer's and operator's public keys through a channel you already trust (a signed
release, a key published by a known party, an exchange inside your own perimeter), and you supply
those keys at verification time. Akmon proves that the holder of a given key signed a given record.
Whether you trust that key is your decision, made outside Akmon.

## Supply chain

- Implemented in Rust.
- A `cargo-deny` gate keeps advisory-bearing crates out of the dependency tree.
- Ed25519 is provided by `ring`, already present in the tree, so the trust layer adds no new
  cryptographic dependency.
- Build inputs are kept reproducible.
- An SBOM and `SHA256SUMS` are published with each release so you can pin and verify what you ran.

## What a verifier must do, in order

Each step is independent. A later step does not rescue an earlier failure.

1. **Integrity first.** Re-hash the objects and rewalk the chain to reproduce the head. If integrity
   fails, stop. Nothing else matters.
2. **Head signature.** If you require provenance, pass `--require-signature` with the trusted signer
   public key. A pass proves which key sealed this head.
3. **Operator attestation.** If you require named accountability, pass `--require-operator` (or
   `--require-operator-key`) with the trusted operator key. A pass proves which operator key claimed
   the session.
4. **Capture level.** If you require a full, replayable recording, pass `--require-capture full`. A
   structural import will fail this check, which is the correct and honest outcome.

The mechanics of running each step, including the standalone verifier and the plain `openssl` path,
are covered in [Verifying evidence (for auditors)](./verifying-evidence.md).

## See also

- [How Akmon works](./architecture.md)
- [Verifying evidence (for auditors)](./verifying-evidence.md)
- [Compliance and evidence](./compliance.md)
- [Glossary](./glossary.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md)
