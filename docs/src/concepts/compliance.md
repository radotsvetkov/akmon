# Compliance and evidence

Documented for Akmon `2.2.0`.

## Who this is for

Compliance, risk, and security teams evaluating whether Akmon helps with record-keeping obligations
for AI agents. This page frames what Akmon does and does not do, and maps its capabilities to a few
common frameworks at a high level. None of it is legal advice.

## Honest framing

Akmon produces durable, signed, independently-verifiable evidence about what an AI agent did. When an
agent changes something you may later have to account for, an Akmon record lets a third party confirm
offline that the record was not altered and which key sealed it. That supports record-keeping
obligations.

It stops there, deliberately. Akmon is not a certification, and it does not guarantee compliance with
any regulation or control. Producing good evidence is one part of a control. The rest, including key
management, control implementation, retention, access governance, and the legal interpretation of
what an obligation requires, belongs to your organization. A verified bundle proves integrity and
key-backed provenance. It does not prove that the agent was correct or that any obligation was met.
See [Trust and threat model](./trust-model.md) for the precise boundaries.

## High-level mapping

The mapping below is descriptive, not a legal claim. It states what Akmon provides and where the
boundary sits. Treat the framework references as orientation, and validate the specifics with your
own teams.

### EU AI Act, Article 12 and Annex IV record-keeping

The EU AI Act places record-keeping and technical-documentation obligations on high-risk AI systems.
The high-risk logging obligations in Article 12, and the technical-documentation expectations in
Annex IV, start applying on 2 August 2026.

- **What Akmon provides.** A portable, content-addressed, hash-linked record of an agent session,
  optionally sealed with an offline Ed25519 signature and an operator-identity attestation. The
  record is tamper-evident and verifiable by a third party offline, including with stock `openssl`.
  That is durable, independently-checkable evidence of what an agent session contained.
- **The boundary.** Akmon does not decide whether your system is high-risk, what must be logged, how
  long records must be retained, or whether your documentation satisfies Annex IV. It does not manage
  your signing keys or establish who holds them. Those determinations and controls are yours.

### NIST AI RMF, MEASURE 2.8

The NIST AI Risk Management Framework's MEASURE function calls for mechanisms to track, document, and
verify AI system behavior, including MEASURE 2.8 on tracking and traceability.

- **What Akmon provides.** A verifiable, traceable record of agent sessions that a reviewer can check
  independently, with an honest `capture_level` that distinguishes a full, replayable recording from
  a structural import. That supports the traceability and documentation MEASURE 2.8 contemplates.
- **The boundary.** Akmon supplies evidence and verification. It does not perform your risk
  measurement, set your thresholds, or judge whether observed behavior is acceptable. The
  measurement program and its conclusions are yours.

### SOC 2, CC7.x and CC8.1

SOC 2's common criteria include monitoring of operations (CC7.x) and change management (CC8.1).

- **What Akmon provides.** Signed, verifiable evidence about agent-driven activity and changes, which
  can feed monitoring and change-management evidence: what an agent session did, recorded
  tamper-evidently, with optional key-backed provenance and operator accountability.
- **The boundary.** Akmon is one evidence source. The design and operating effectiveness of your
  controls, the completeness of your monitoring, the governance of who reviews what, and the
  management of the keys that make a signature meaningful are all your responsibility. Auditor
  acceptance of any evidence is determined in your audit, not by Akmon.

## Capture honesty

Compliance use depends on not overstating what was recorded. Akmon labels every record with a
`capture_level`:

- A session run under Akmon's own bundled reference agent is `full`: a complete, deterministically
  replayable recording.
- A session brought in through an OpenTelemetry import is `structural`: the shape of the session, not
  a complete recording. It cannot be replayed, and `akmon bundle verify --require-capture full` fails
  on it.

Require the capture level your obligation actually needs, and do not present a structural import as a
full recording.

## Validate with your own legal and compliance teams

Akmon helps you produce evidence. It does not interpret the law and it does not certify you. Whether
a given record satisfies a given obligation, how long you must retain records, how you govern and
protect signing keys, and how you implement and evidence the surrounding controls are decisions for
your own legal and compliance teams. Validate any use of Akmon for a regulatory or audit purpose with
those teams before you rely on it.

## See also

- [Trust and threat model](./trust-model.md)
- [How Akmon works](./architecture.md)
- [Verifying evidence (for auditors)](./verifying-evidence.md)
- [Regulated reviewer flow](./reviewer-flow.md)
- [Glossary](./glossary.md)
