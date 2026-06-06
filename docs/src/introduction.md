# Introduction

Akmon is a tamper-evident evidence and verification layer for AI agents.
It sits on top of whatever agent you already run, through OpenTelemetry or with Akmon's own reference agent.
Every session becomes a portable, content-addressed, cryptographically signed record that someone else can verify for themselves.

The point that matters most: a third party can check a signature offline with nothing but `openssl`, with no Akmon install and no cloud service.
That is the whole pitch. When an AI agent changes something, you may have to prove later what it actually did, to a regulator or auditor who does not trust you and does not run your tools.
Under the EU AI Act, the high-risk logging obligations in Article 12 and Annex IV start to apply on 2 August 2026.

Akmon is producer-agnostic. The verification chain is the product, not the agent.
This page explains the problem Akmon is designed to solve, the design choices behind it, what ships in v2.x (latest **v2.2.0**, AGEF v0.1.3), and where Akmon is intentionally not trying to compete.

## v2.2.0 highlights

v2.2.0 is the trust-layer release. It turns Akmon into a producer-agnostic evidence and verification layer and makes that claim provable end to end. The toolchain:

- Import any agent: `akmon otel import <trace.json>` turns an OpenTelemetry GenAI trace into an AGEF session. It reads the current v1.37 structured form and the older v1.36-and-earlier message-event form that most deployed agents still emit. The capture level is recorded honestly (imports are `structural`, never dressed up as a full recording).
- Generate a key: `akmon bundle keygen --out k.pk8 --public-out k.pub` creates an Ed25519 signing key (PKCS#8 v2).
- Sign: `akmon bundle sign <bundle> --key k.pk8` adds an offline Ed25519 signature over the session head.
- Attest an operator: `akmon bundle attest <bundle> --key op.pk8 --operator-id you@org --role approver` records the accountable person behind a session.
- Verify: `akmon bundle verify <bundle> --verify-key k.pub --require-signature` checks integrity, the signature, any operator attestation, and the capture level.
- Prove with openssl: `akmon bundle prove-openssl <bundle> --verify-key k.pub --out-dir proof` writes the statement, signature, and public key so anyone can check the signature with plain `openssl`.
- Verify on its own: `agef-verify <bundle> --verify-key k.pub` is a small, separate binary for auditors that does not need the full Akmon install.

See the [release notes](./releases/v2.2.0.md) and the walkthrough, [from an OTEL trace to an offline openssl proof](./tutorials/otel-to-openssl-walkthrough.md).

## The problem Akmon was built to solve

When an AI agent changes something, you may have to prove later what it actually did.
The person asking could be a regulator, a security reviewer, or an incident team, and it might be years after the fact.
They may not trust you, and they may not run your tools.
The blocker is usually not raw model capability. The blocker is evidence quality.

Most agent telemetry cannot stand up to that.
It lives in process memory, or in mutable, unsigned spans, and "the AI did it" is not an answer anyone will accept.
Teams repeatedly run into the same questions:

- What exactly did the agent read?
- What tools did it call?
- What side effects happened on disk or in shell?
- Which policy decision allowed each side effect?
- Can a third party verify the record's integrity and authorship without trusting you?

In many tools, those answers are partial or ephemeral.
You get a useful session in the moment, but weak forensic value later.
That gap is acceptable for low-risk prototyping.
It is a hard stop for regulated release workflows.

Akmon treats the evidence itself as the thing you ship.
It takes a session, either your own or one from any OpenTelemetry-instrumented agent, and turns it into a sealed record.
Someone else can then check that record's integrity and authorship independently, with standard tools, even on a machine that has never heard of Akmon.

Provider lock-in is the second recurring failure mode.
Model quality, latency, legal terms, and cost change over time.
If your agent workflow depends on one provider's roadmap, your engineering process inherits that business risk.
Regulated teams and enterprise teams often need optionality: local models for sensitive code paths, hosted models for throughput, and explicit controls over where prompts go.

Operational portability is the third issue.
Real engineering happens in mixed environments:

- local laptops,
- remote SSH sessions,
- CI runners,
- hardened enterprise hosts,
- restricted network segments.

If the agent requires a specific IDE plugin stack or heavyweight runtime chain, adoption collapses outside a narrow desktop workflow.
Akmon is built for those constraints first.

## The design decisions (and why)

### Single binary

Akmon ships as a standalone Rust binary.
That has practical effects beyond install convenience.

- Runtime state is explicit and portable.
- Environment drift is reduced relative to dynamic plugin/runtime stacks.
- CI and remote-host deployment stay simple.

If two machines run the same Akmon version, behavior is easier to reason about and support.
Troubleshooting tends to focus on policy, provider config, repository state, or model behavior rather than host runtime mismatch.

### Bring your own key / bring your own model

Akmon supports Anthropic, OpenAI, OpenRouter, Groq, Azure OpenAI, Bedrock, OpenAI-compatible endpoints, and Ollama.
Model selection remains an operator decision.

That matters for:

- commercial leverage,
- legal and data-boundary control,
- outage resilience,
- per-task cost/performance tuning.

The objective is not to force one "best model."
The objective is to keep model strategy decoupled from tooling adoption.

### Typed permission boundaries

Akmon treats tool operations as explicit capabilities, not implicit side effects.
Reads, writes, shell execution, and network actions are mediated through policy and approval flow.

This creates a clear boundary:

- the model can request an action,
- the runtime can enforce policy,
- the operator can review and approve or deny.

That boundary is critical in environments where side effects must be reviewable and explainable.

### Session evidence as a first-class output

Most tools treat logs as support artifacts.
Akmon treats the session artifact as a product output.

A useful AI run is not just "did it produce code."
A useful AI run is "can we verify what happened and carry that evidence through review, CI, and audit."

### Context discipline

Akmon encourages explicit context shaping rather than perpetual thread growth.
Teams typically get better outcomes when they separate work into:

1. exploration,
2. planning/specification,
3. implementation and verification.

This reduces context drift and makes outcomes easier to reproduce.

## The evidence and verification model

Akmon records each session as a content-addressed event journal with cryptographic chain integrity.
That gives a reviewer a concrete object to inspect instead of reconstructing behavior from partial logs.
The session, whether Akmon produced it or it came in from another agent's trace, is then exported as a portable AGEF bundle that can be signed and verified anywhere.

At a high level:

- Events are linked in order and integrity-checked.
- A bundle can carry an offline Ed25519 signature over the session head, and an operator attestation that records the accountable person.
- A third party can verify integrity and authorship with `agef-verify`, or with plain `openssl` after `akmon bundle prove-openssl`, with no Akmon install required.
- Two sessions can be compared structurally and at field level.
- Sessions Akmon produced itself can be replayed deterministically against recorded providers and tools.

Akmon implements AGEF v0.1.3 as a practical reference implementation for portable AI-agent session evidence.
v0.1.3 adds two optional pieces on top of the hash chain: offline Ed25519 signatures and operator attestations. Both are optional, so a plain bundle stays small and older readers keep working.
The goal is operational interoperability and independent verifiability, not vendor-specific lock-in.

### Command surface

The verification chain is the core:

- `otel import` to bring in any OpenTelemetry GenAI trace (v1.37 structured and the legacy v1.36-and-earlier message-event form),
- `bundle keygen`, `bundle sign`, and `bundle attest` to produce a key, sign the head, and record an operator,
- `bundle verify`, `agef-verify`, and `bundle prove-openssl` for integrity, signature, attestation, and capture checks, including offline `openssl` proof,
- `bundle export` / `bundle import`, `inspect`, `diff`, and `redact` for portable and sanitized handoff,
- `chat` / `--task` for Akmon's own reference agent, with `audit`, `evidence`, `verify`, and `replay` for full-capture sessions.

These commands are meant to compose.
A common pattern for an imported session is:

1. import a trace with `otel import`,
2. export it as a bundle and sign it,
3. verify integrity, signature, and capture level,
4. emit an `openssl` proof a stranger can check.

## Who Akmon is for

Akmon targets teams that must prove what AI did, not just benefit from what AI suggested.

### Aerospace and avionics teams

For organizations working under DO-178C-style qualification and evidence pressure, session traceability and deterministic replay reduce ambiguity during review.

### Medical device software teams

For IEC 62304-oriented development, controlled side effects and audit-ready artifacts support stronger change documentation and risk controls.

### Automotive software teams

For ISO 26262-influenced workflows, reproducible agent behavior and explicit evidence chains improve confidence in AI-assisted modifications.

### Finance and enterprise controls teams

For SOC 2 or similar control environments, the session artifact model helps demonstrate governance over AI-driven code changes.

### Defense and high-assurance environments

For CMMC-style and restricted-network contexts, single-binary deployment, policy boundaries, and provider flexibility are practical adoption requirements.

### Platform and SRE teams

For teams running large automation surfaces in CI, structured outputs and verifiable artifacts make autonomous tasks easier to gate and monitor.

## What Akmon is intentionally not

Akmon is opinionated about scope.
That includes clear non-goals.

- It is not trying to replace IDE-native completion workflows.
- It is not optimized for maximum "chat polish" over evidentiary rigor.
- It is not tied to a single model provider's product strategy.
- It is not built around opaque, non-replayable agent behavior.

Those tradeoffs are deliberate.
Akmon prioritizes reviewability, operational control, and portability over broad UX coverage.

## Practical usage guidance

### Use model tiers intentionally

Use lower-cost models for exploration and mechanical edits.
Use stronger models for architecture or multi-file reasoning.
This keeps cost predictable without forcing one model for every task.

### Keep project context explicit

Maintain `AKMON.md` with constraints, architecture notes, and decision boundaries.
High-quality local context usually improves output quality more than longer ad-hoc prompts.

### Treat evidence generation as part of done

In regulated workflows, task completion includes evidence readiness.
A run is not complete until required verification and artifact checks pass.

### Gate automation with policy and verification

For headless workflows, use explicit policy, budget limits, and integrity checks so autonomous runs fail closed when constraints are violated.

## Adoption notes for regulated teams

Teams adopting Akmon in regulated contexts usually move in phases instead of switching everything at once.

### Phase 1: Observe

Start by running scoped tasks with full session capture enabled.
Focus on understanding evidence quality and policy fit before broad automation.

### Phase 2: Constrain

Introduce tighter policy defaults and approval rules for writes, shell, and network operations.
Treat denied operations as useful feedback about control boundaries, not as friction to bypass.

### Phase 3: Verify

Standardize post-run verification steps in CI and review checklists.
Require session integrity checks for categories of changes where auditability is mandatory.

### Phase 4: Operationalize

Package repeatable workflows for common engineering tasks and gate them with policy and evidence requirements.
The goal is consistent, reviewable operation rather than maximal autonomy.

This phased approach keeps rollout practical:

- engineers get immediate utility,
- governance teams get deterministic evidence,
- reliability standards are raised without blocking delivery.

## Where to go next

- Install and first run: [Getting Started](./getting-started/installation.md)
- Headless automation: [Headless mode](./usage/headless.md)
- Interactive usage: [Interactive mode](./usage/interactive.md)
- Policy controls: [Policy profiles](./features/policy-profiles.md)
- Core terms: [Glossary](./concepts/glossary.md)
- Reviewer handoff: [Regulated reviewer flow](./concepts/reviewer-flow.md)
- Session comparison: [Session diff reference](./reference/diff.md)
- Architecture for contributors: [Contributing architecture](./contributing/architecture.md)
