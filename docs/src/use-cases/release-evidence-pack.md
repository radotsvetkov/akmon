# Assemble a signed evidence pack for a regulated release

Documented for Akmon `2.2.0`.

## Who this is for

A release or compliance owner who must hand a reviewer or auditor a defensible, independently verifiable record of the AI-assisted work that went into a regulated release. This page walks the full assembly: gather the relevant sessions, export and sign them, capture operator sign-off, and verify with the requirements your obligation actually needs, so the pack a reviewer receives can be checked offline without trusting your tooling.

## What you end up with

- One signed `.akmon` bundle per relevant session, sealed with an Ed25519 head signature.
- An operator attestation on each bundle recording who approved it, backed by a key.
- A verification result that holds the signature present and, where required, the capture level full.
- A pack a reviewer can verify independently with Akmon, the standalone `agef-verify`, or plain `openssl`.

## Before you start

Create a signing key once and publish the public half to reviewers out of band.

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub.hex
```

Keep `signer.pk8` secret. The reviewer trusts the public key, not your machine, so the channel you use to hand over `signer.pub.hex` is what gives the signature meaning.

## Steps

1. Produce or import the relevant sessions.

For work done by Akmon's own reference agent, run the task and note the session id; this is a full-capture, replayable recording.

```bash
akmon --yes --output json --task "implement and test the release change" | tee run.json
SESSION_ID="$(jq -r '.session_id' run.json)"
```

For work done by another agent that is OpenTelemetry-instrumented, import its trace; this is a structural record, the shape of the session and not a full recording.

```bash
akmon otel import /path/to/trace.json --journal ./journal --format json
```

2. Export each session as a portable bundle.

```bash
akmon bundle export "${SESSION_ID}" --output "${SESSION_ID}.akmon"
```

3. Sign the bundle's head offline.

```bash
akmon bundle sign "${SESSION_ID}.akmon" --key signer.pk8
```

4. Attest operator sign-off, binding the approver's key and role to the session.

```bash
akmon bundle attest "${SESSION_ID}.akmon" \
  --key operator.pk8 \
  --operator-id approver@example.com \
  --role approver
```

See [Record who approved an AI change](./operator-sign-off.md) for the operator key setup and the honest boundary: the attestation proves the holder of the operator key approved the session, not that the named person did. Trust attaches to the key, established out of band.

5. Verify the pack with the requirements the obligation needs. Require a present, valid signature, and require full capture only where the obligation calls for a complete recording.

```bash
akmon bundle verify "${SESSION_ID}.akmon" \
  --verify-key signer.pub.hex --require-signature \
  --operator-key operator.pub.hex --require-operator-key operator.pub.hex \
  --require-capture full
```

A full-capture reference-agent session passes `--require-capture full`. A structural OTEL import fails that check, which is the honest result; for those sessions, drop `--require-capture full` and treat the bundle as structural evidence rather than a replayable recording. Do not present a structural import as a full recording.

## What the reviewer receives and how they check it

Hand the reviewer the signed `.akmon` bundles and the public keys (`signer.pub.hex`, and `operator.pub.hex` if accountability matters). The reviewer establishes key trust through their own process and then verifies independently, with no need to trust your machine:

- With Akmon: `akmon bundle verify <bundle> --verify-key signer.pub.hex --require-signature`.
- Without the full agent: `agef-verify <bundle> --verify-key signer.pub.hex --require-signature`.
- With nothing but openssl: you emit `akmon bundle prove-openssl <bundle> --verify-key signer.pub.hex --out-dir proof`, and the reviewer runs the printed `openssl pkeyutl -verify ...` command (OpenSSL 3.x; macOS LibreSSL cannot verify Ed25519).

The full reading guide for outcomes (verified, invalid, `unverified_no_key`, unattributed, structural versus full) is in [Verify evidence on an air-gapped machine](./air-gapped-audit.md).

## The honest boundary

A verified pack proves integrity and key-backed provenance: the records were not altered and the holders of the named keys sealed and approved them. It does not prove the agent was correct, that any person holds a key, or that any regulatory obligation was met. Akmon helps you produce evidence for obligations such as EU AI Act Article 12 and Annex IV record-keeping, NIST AI RMF MEASURE 2.8, and SOC 2 CC7.x and CC8.1, but it is not a certification and does not guarantee compliance. Require the capture level your obligation actually needs, and validate any regulatory use with your own legal and compliance teams. See [Compliance and evidence](../concepts/compliance.md) for the precise boundary.

## See also

- [Compliance and evidence](../concepts/compliance.md)
- [Regulated reviewer flow](../concepts/reviewer-flow.md)
- [akmon bundle export](../reference/bundle-export.md)
- [akmon bundle sign](../reference/sign.md)
- [akmon bundle attest](../reference/bundle-attest.md)
- [akmon bundle verify](../reference/bundle-verify.md)
- [Record who approved an AI change](./operator-sign-off.md)
- [Verify evidence on an air-gapped machine](./air-gapped-audit.md)
