# How Akmon relates to other tooling

Akmon is not a competitor to the coding agent you already run. It is an evidence and verification layer that sits on top of one. The useful question is not "Akmon or tool X." It is "what does Akmon add to a stack that already has an agent, and where does it overlap with governance tooling you may already use."

This page places Akmon against two neighbors that people reasonably confuse it with: the agents that produce sessions, and the governance and ledger systems that try to make those sessions trustworthy. Where a vendor leads, this page says so.

## The layer Akmon occupies

Most agent tooling is a producer. It runs a model, calls tools, and changes things. Its telemetry is usually built for live observability: traces and spans that help you debug a run while it is fresh.

Akmon is a consumer and a verifier. It takes a session, either from its own bundled reference agent or from any OpenTelemetry-instrumented agent through `akmon otel import`, and turns it into a portable, content-addressed, hash-linked AGEF record. That record can be signed offline with Ed25519, can carry a separately signed operator attestation, and can be verified by a third party with no Akmon install and no cloud, using `agef-verify` or stock `openssl` after `akmon bundle prove-openssl`.

So the comparison is not feature-for-feature against an agent. It is: does the record survive contact with someone who does not trust you and does not run your tools, possibly years later. That is the gap Akmon fills.

## Relation to agent and coding tooling

Akmon is producer-agnostic by design. Its own bundled coding agent is the reference, gold-fidelity producer (it records `full` capture and replays deterministically), but it is not the headline. The headline is the verification chain, and that chain accepts sessions from agents Akmon did not write.

| Dimension | Akmon (evidence and verification layer) | Typical coding or agent tool |
| --- | --- | --- |
| Primary role | Records, signs, and verifies sessions as evidence | Produces sessions by running a model and tools |
| Output you keep | Portable, signed, content-addressed AGEF bundle | Live trace or chat transcript, often ephemeral |
| Third-party verification | Offline, signature-checked, no Akmon needed (`openssl`/`agef-verify`) | Usually requires the vendor's stack to interpret |
| Producer coupling | Producer-agnostic via OpenTelemetry import | The agent is the product |
| Capture honesty | Records `full` or `structural` explicitly; never overstates | Varies; replay fidelity often unstated |

If you already have an agent you like, Akmon does not ask you to replace it. It asks for its OpenTelemetry trace, and gives you back a record you can prove later.

## Relation to governance and ledger tooling: a fair Microsoft comparison

The closest neighbors to Akmon are not coding agents. They are the systems that try to make agent activity auditable. Microsoft ships the most prominent ones, so it is worth being precise and fair about where each fits.

- The Microsoft Agent Governance Toolkit uses a hash chain with HMAC. That gives tamper-evidence within a trust domain that holds the shared secret, but HMAC is symmetric: anyone who can verify can also forge, and there is no standalone verifier a distrusting third party can run. Akmon uses an asymmetric Ed25519 signature over the session head, so a verifier checks authorship with a public key and cannot forge a new one. Akmon also ships `agef-verify` and the `openssl` proof path for verification with no Akmon install.
- Azure Confidential Ledger does provide signed, tamper-evident records, but it is Azure-locked and not agent-aware. The trust anchor is the Azure service. Akmon's bundle is cloud-independent and agent-aware: the record is a portable file, the signature is verifiable offline, and the format models agent sessions (events, tool calls, capture level, operator) rather than generic ledger entries.
- Microsoft Foundry's own documentation states that its traces cannot support full replay. Akmon distinguishes capture levels explicitly: a reference-agent session records `full` capture and replays deterministically, while an OpenTelemetry import records `structural` capture, and `akmon replay` refuses to claim a structural import is replayable.

Where Microsoft leads, plainly: distribution, ecosystem integration, and the gravitational pull of an existing Azure footprint. If your organization is standardized on Azure and Microsoft's agent stack, those tools meet you where you already are, and Akmon is complementary rather than a replacement. Akmon's contribution is the portable, signed, cloud-independent, offline-verifiable record on top of whatever you run. The two layers compose: produce and govern in your platform of choice, then seal the session into a record a regulator or counterparty can verify without trusting that platform.

## Where this matters

When an AI agent changes something, you may later have to prove what it did, to a regulator, an auditor, or an incident team who does not trust you and does not run your tools. Under the EU AI Act, the high-risk logging obligations in Article 12 and Annex IV start applying on 2 August 2026. Akmon helps you produce evidence for that kind of obligation, and for NIST AI RMF (MEASURE 2.8) and SOC 2 (CC7.x, CC8.1) workflows. It is not a certification and does not by itself guarantee compliance; validate fit with your own legal and compliance teams.

## Choosing what to use

- Keep your agent. Use Akmon to import its trace and produce a verifiable record. Different teams can standardize on different agents and still hand back the same kind of evidence.
- If you are deep in Azure and Microsoft's governance stack, keep it, and use Akmon as the portable, offline-verifiable layer on top for records that must leave that trust domain.
- Use Akmon's own bundled reference agent when you want gold-fidelity `full` capture and deterministic replay, not because it is trying to win on coding UX.

The realistic stack is layered, not exclusive: an agent to do the work, a governance platform if you have one, and Akmon to turn the result into something a stranger can verify.

## Common mistakes

- Comparing Akmon to a coding agent on response quality. That is the producer's job; Akmon verifies the record, whatever produced it.
- Assuming HMAC-chained logs give third-party non-repudiation. They do not; symmetric verification is also symmetric forgery.
- Treating a `structural` OTEL import as a full recording. It is an honest transcription, not a replayable capture.
- Deferring compliance and deployment fit until late adoption, then discovering the evidence does not verify outside your own tools.

[Introduction](./introduction.md) and [Security model](./features/security.md).
