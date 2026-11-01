# Assemble a signed evidence pack for a regulated release

Documented for Akmon `3.0.0`.

## Who this is for

A release or compliance owner who must hand a reviewer or auditor a defensible, independently verifiable record of the AI-assisted work that went into a regulated release. This page walks the full assembly: gather the relevant sessions, seal them with operator sign-off, and verify with the requirements your obligation actually needs, so the pack a reviewer receives can be checked offline without trusting your tooling.

## What you end up with

- One signed `.akmon` bundle per relevant session, sealed with an Ed25519 head signature.
- An operator attestation on each bundle recording who approved it, backed by a key.
- A verification result that holds the signature present and, where required, the capture level full.
- A pack a reviewer can verify independently with Akmon, the standalone `agef-verify`, or plain `openssl`.

## Before you start

Create a signing key once and publish the public half to reviewers out of band.

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub
```

Keep `signer.pk8` secret. The reviewer trusts the public key, not your machine, so the channel you use to hand over `signer.pub` is what gives the signature meaning.

## Steps

1. Produce or import the relevant sessions.

For work done by Akmon's own reference agent, run the task and note the session id; this is a full-capture, replayable recording.

```bash
akmon --yes --output json --task "implement and test the release change" | tee run.json
SESSION_ID="$(jq -r '.session_id' run.json)"
```

For work done by another agent that is OpenTelemetry-instrumented, export its trace as OTLP/JSON. Its capture level (`full`, `partial`, or `structural`) depends on whether the instrumentation recorded message content.

2. Seal each session with the signing key and the approver's operator key. `akmon seal` exports, signs, attests, and verifies in one step, and writes the public keys next to the bundle.

```bash
akmon seal "${SESSION_ID}" \
  --key signer.pk8 \
  --operator-key operator.pk8 --operator-id approver@example.com --role approver \
  --output "release/${SESSION_ID}.akmon"

akmon seal /path/to/trace.json \
  --key signer.pk8 \
  --operator-key operator.pk8 --operator-id approver@example.com --role approver \
  --output release/other-agent.akmon
```

3. Timestamp each bundle's signed statement with an RFC 3161 authority if the reviewer needs to know when it was sealed (see [Trusted timestamps](../guides/trusted-timestamps.md)).

See [Record who approved an AI change](./operator-sign-off.md) for the operator key setup and the honest boundary: the attestation proves the holder of the operator key approved the session, not that the named person did. Trust attaches to the key, established out of band.

4. Verify the pack with the requirements the obligation needs. Require a present, valid signature, and require full capture only where the obligation calls for a complete recording.

```bash
akmon bundle verify "${SESSION_ID}.akmon" \
  --verify-key signer.pub --require-signature \
  --operator-key operator.pub --require-operator-key operator.pub \
  --require-capture full
```

A reference-agent session passes `--require-capture full`. An imported trace passes only if it carried all message content; a `partial` or `structural` import fails, which is the honest result. For those, drop `--require-capture full` and present the bundle at its recorded capture level.

## What the reviewer receives and how they check it

Hand the reviewer the signed `.akmon` bundles and the public keys (`signer.pub`, and `operator.pub` if accountability matters). The reviewer establishes key trust through their own process and then verifies independently, with no need to trust your machine:

- With Akmon: `akmon bundle verify <bundle> --verify-key signer.pub --require-signature`.
- Without the full agent: `agef-verify <bundle> --verify-key signer.pub --require-signature`.
- With nothing but openssl: you emit `akmon bundle prove-openssl <bundle> --verify-key signer.pub --out-dir proof`, and the reviewer runs the printed `openssl pkeyutl -verify ...` command (OpenSSL 3.x; macOS LibreSSL cannot verify Ed25519).

The full reading guide for outcomes (verified, invalid, `unverified_no_key`, unattributed, structural versus full) is in [Verify evidence on an air-gapped machine](./air-gapped-audit.md).

## The honest boundary

A verified pack proves integrity and key-backed provenance: the records were not altered and the holders of the named keys sealed and approved them. It does not prove the agent was correct, that any person holds a key, or that any regulatory obligation was met. Akmon helps you produce evidence for obligations such as the EU AI Act's logging and documentation requirements (Art. 12 and Annex IV point 2(g), applicable to high-risk systems from 2 December 2027 or 2 August 2028), NIST AI RMF, and SOC 2, but it is not a certification and does not make anything compliant. Require the capture level your obligation actually needs, and validate any regulatory use with your own legal and compliance teams. See the [Compliance crosswalk](../concepts/compliance.md) for the precise boundary. Keep the pack in immutable storage for as long as your obligations require ([Retention and archiving](../guides/retention.md)).

## See also

- [Compliance crosswalk](../concepts/compliance.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
- [akmon bundle export](../reference/bundle-export.md)
- [akmon seal](../reference/seal.md)
- [akmon bundle attest](../reference/bundle-attest.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [Record who approved an AI change](./operator-sign-off.md)
- [Verify evidence on an air-gapped machine](./air-gapped-audit.md)
