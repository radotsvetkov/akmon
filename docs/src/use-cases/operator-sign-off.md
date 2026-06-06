# Record who approved an AI change

Documented for Akmon `2.2.0`.

## Who this is for

An operator or approver who has to put their name, backed by a key, behind an AI agent session. If your process needs a human to sign off on what an agent did, for example to satisfy a human-oversight requirement such as EU AI Act Article 14, this page shows how to attach a key-backed operator attestation to a bundle and how a verifier later confirms it offline.

The session itself is produced separately (by Akmon's reference agent or imported from an OpenTelemetry trace) and exported to a `.akmon` bundle. This use case is about the sign-off layered on top of that bundle.

## What you end up with

- An operator key pair that identifies you as the approver.
- A `.akmon` bundle carrying a signed `AGEF-OPERATOR-v1` attestation that binds your `operator_id` and `role` to the session head.
- A public key a verifier can use, out of band, to confirm that the holder of your operator key approved the session, with `akmon bundle verify --require-operator-key`.

## The honest boundary: trust attaches to the key, not the name

An operator attestation is a signature over four self-asserted identity fields (`operator_id`, `display_name`, `role`, `org`) plus the session head. Verifying it proves only that the holder of a particular private key signed those fields. It does not prove that the person is who the `operator_id` string says. The name is a claim; the key is the trust anchor.

A verifier establishes which key belongs to which person out of band, through your own process: an HR directory, a signed roster, a key-distribution ceremony. Only after a verifier has decided to trust a specific key does the self-asserted name carry weight. Akmon never asserts the name is true on its own.

What this does prove: the holder of operator key `K` approved this exact session, with the role they stated, and the attestation is bound to this bundle's head so it cannot be transplanted onto a different session.

What it does not prove: that the named person physically pressed a button, that the agent's work was correct, or that any obligation was met. Those are out of band.

## Steps

1. Generate an operator key. Keep the private key secret; you will distribute only the public key.

```bash
akmon bundle keygen --out operator.pk8 --public-out operator.pub.hex
```

2. Attest the bundle with your operator id and role. The attestation is appended in place and is purely additive: the event hash chain and any existing head signature are left byte-untouched.

```bash
akmon bundle attest /path/to/session.akmon \
  --key operator.pk8 \
  --operator-id approver@example.com \
  --display-name "A. Approver" \
  --role approver \
  --org "Example Corp"
```

3. Distribute `operator.pub.hex` to verifiers out of band, through the channel your process trusts. This step is what makes the name meaningful later; without it a verifier only sees a self-asserted string.

4. The verifier requires that specific operator key to have a valid attestation.

```bash
akmon bundle verify /path/to/session.akmon \
  --operator-key operator.pub.hex \
  --require-operator-key operator.pub.hex
```

This exits `0` only when the named key has a cryptographically valid attestation on the bundle. With `--require-operator` instead, any one trusted operator key satisfies the gate; with `--require-operator-key` the gate names a specific key. Without any trusted key supplied, an attested bundle reports `unverified_no_key`, which is informational rather than a failure. A verifier who does not run Akmon can use the standalone `agef-verify` with the same flags.

## Combine with a head signature

Operator sign-off answers "who approved this". The bundle's head signature answers "is the record intact and from a trusted producer". They are independent and complementary. A typical sealed handoff requires both:

```bash
akmon bundle verify /path/to/session.akmon \
  --verify-key signer.pub.hex --require-signature \
  --operator-key operator.pub.hex --require-operator-key operator.pub.hex
```

## See also

- [akmon bundle attest](../reference/bundle-attest.md)
- [akmon bundle keygen](../reference/bundle-keygen.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [Trust and threat model](../concepts/trust-model.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
- [Verify evidence on an air-gapped machine](./air-gapped-audit.md)
