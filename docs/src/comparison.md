# How Akmon relates to other tooling

Akmon is not a competitor to the coding agent you already run. It is an evidence and verification layer that sits on top of one. The useful question is not "Akmon or tool X." It is "what does Akmon add to a stack that already has an agent, and where does it overlap with governance tooling you may already use."

This page places Akmon against two neighbors that people reasonably confuse it with: the agents that produce sessions, and the governance and ledger systems that try to make those sessions trustworthy. Where a vendor leads, this page says so.

## The layer Akmon occupies

Most agent tooling is a producer. It runs a model, calls tools, and changes things. Its telemetry is usually built for live observability: traces and spans that help you debug a run while it is fresh.

Akmon is a consumer and a verifier. It takes a session, either from its own bundled reference agent or from any OpenTelemetry-instrumented agent through `akmon seal trace.json`, and turns it into a portable, content-addressed, hash-linked AGEF record. That record can be signed offline with Ed25519, can carry a separately signed operator attestation, and can be verified by a third party with no Akmon install and no cloud, using `agef-verify` or stock `openssl` after `akmon bundle prove-openssl`.

So the comparison is not feature-for-feature against an agent. It is: does the record survive contact with someone who does not trust you and does not run your tools, possibly years later. That is the gap Akmon fills.

## Relation to agent and coding tooling

Akmon is producer-agnostic by design. Its own bundled coding agent is the full-capture reference producer (it records `full` capture and replays deterministically), but it is not the headline. The headline is the verification chain, and that chain accepts sessions from agents Akmon did not write.

| Dimension | Akmon (evidence and verification layer) | Typical coding or agent tool |
| --- | --- | --- |
| Primary role | Records, signs, and verifies sessions as evidence | Produces sessions by running a model and tools |
| Output you keep | Portable, signed, content-addressed AGEF bundle | Live trace or chat transcript, often ephemeral |
| Third-party verification | Offline, signature-checked, no Akmon needed (`openssl`/`agef-verify`) | Usually requires the vendor's stack to interpret |
| Producer coupling | Producer-agnostic via OpenTelemetry import | The agent is the product |
| Capture honesty | Records `full`, `partial`, or `structural` inside the signed record | Varies; replay fidelity often unstated |

If you already have an agent you like, Akmon does not ask you to replace it. It asks for its OpenTelemetry trace, and gives you back a record you can prove later.

## Relation to governance and ledger tooling (as of November 2026)

The closest neighbours to Akmon are systems that make agent activity auditable. Microsoft ships the most prominent ones. Vendor products change quickly, so treat this section as dated and check the current documentation before relying on it.

- **Microsoft Agent Governance Toolkit.** Its documentation now describes a Merkle-chained, tamper-evident audit log (SHA-256 hash chaining with inclusion proofs). That is real tamper-evidence. Microsoft's own SOC 2 mapping for the toolkit rates processing integrity as partial ([source](https://microsoft.github.io/agent-governance-toolkit/compliance/soc2-mapping/)). As of November 2026 we found no documented per-entry asymmetric signature on its audit log that a stranger can check with only a public key, and no standalone third-party verifier; check its current documentation, which may have changed. What Akmon offers: an asymmetric Ed25519 signature over the session head, so verifiers hold only a public key and cannot forge a record; a standalone verifier (`agef-verify`) a distrusting third party can run; a proof checkable with stock OpenSSL 3; and no dependency on any cloud or service at verification time.
- **Azure Confidential Ledger.** Provides signed, tamper-evident records with receipts that can be checked against the ledger's service identity. It is a general-purpose ledger tied to an Azure service, not an agent-session format. An Akmon bundle is a portable file that models agent sessions (events, provider and tool calls, capture level, operator) and verifies with no service involved.
- **Microsoft Foundry tracing.** Built for observability inside the vendor's service. As of November 2026 we found no documented way to export a Foundry trace as an asymmetrically signed record that a third party can verify offline. Akmon distinguishes capture levels explicitly and records them inside the signed record; only sessions from Akmon's own reference agent are replayable.

Where Microsoft leads: distribution, ecosystem integration, and an existing Azure footprint. If you are standardised on Microsoft's agent stack, keep it. Akmon is complementary: produce and govern in your platform, then seal the session into one file a regulator or counterparty can verify without trusting that platform.

## Where this matters

When an AI agent changes something, you may later have to prove what it did, to a regulator, an auditor, or an incident team who does not trust you and does not run your tools. Under the EU AI Act as amended by the Digital Omnibus on AI, the high-risk logging obligations (Art. 12, with retention under Art. 19 and 26(6)) apply from 2 December 2027 for Annex III systems and 2 August 2028 for Annex I systems; DORA and NIS2 logging rules already apply (the NIS2 Implementing Regulation (EU) 2024/2690 to the digital-infrastructure and ICT-service entity types it lists, other NIS2 entities through national measures). Akmon helps you produce evidence for that kind of obligation. It is not a certification and does not by itself make anything compliant; see the [Compliance crosswalk](./concepts/compliance.md) and validate fit with your own legal and compliance teams.

## Choosing what to use

- Keep your agent. Use Akmon to import its trace and produce a verifiable record. Different teams can standardize on different agents and still hand back the same kind of evidence.
- If you are deep in Azure and Microsoft's governance stack, keep it, and use Akmon as the portable, offline-verifiable layer on top for records that must leave that trust domain.
- Use Akmon's own bundled reference agent when you want `full` capture and deterministic replay, not because it is trying to win on coding UX.

The realistic stack is layered, not exclusive: an agent to do the work, a governance platform if you have one, and Akmon to turn the result into something a stranger can verify.

## Common mistakes

- Comparing Akmon to a coding agent on response quality. That is the producer's job; Akmon verifies the record, whatever produced it.
- Assuming a tamper-evident log gives third-party non-repudiation. Without an asymmetric signature and a verifier the third party can run themselves, it does not.
- Treating an OpenTelemetry import as a replayable recording. It is an honest transcription of the trace at its recorded capture level.
- Deferring compliance and deployment fit until late adoption, then discovering the evidence does not verify outside your own tools.

[Introduction](./introduction.md) and [Security model](./features/security.md).
