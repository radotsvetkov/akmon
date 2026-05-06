# Introduction

Akmon is the review-aware AI coding agent for regulated engineering.
It is built for teams where "the model suggested it" is not enough evidence to merge, release, or certify software changes.
Every session is captured as a tamper-evident, content-addressed artifact that can be replayed, compared, and verified later.

Akmon is intentionally not a "best autocomplete UX" product.
It is a terminal-first control plane for AI-assisted code change work where traceability, deterministic evidence, and explicit operator control are non-negotiable.
The core question is simple: when an auditor, reviewer, or incident responder asks what happened, can you prove it?

This page explains the problem Akmon is designed to solve, the design choices behind it, what ships in v2.0.0, and where Akmon is intentionally not trying to compete.

## The problem Akmon was built to solve

AI coding agents can now produce meaningful code changes, but many environments still cannot rely on them for critical work.
The blocker is usually not raw model capability.
The blocker is evidence quality.

Teams repeatedly run into the same questions:

- What exactly did the agent read?
- What tools did it call?
- What side effects happened on disk or in shell?
- Which policy decision allowed each side effect?
- Can we replay the run and validate that the artifact is still intact?

In many tools, those answers are partial or ephemeral.
You get a useful session in the moment, but weak forensic value later.
That gap is acceptable for low-risk prototyping.
It is a hard stop for regulated release workflows.

Provider lock-in is the second recurring failure mode.
Model quality, latency, legal terms, and cost change over time.
If your coding workflow depends on one provider's roadmap, your engineering process inherits that business risk.
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

## The session evidence model in v2.0.0

v2.0.0 centers Akmon around a deterministic session evidence workflow.
The system records each run as a content-addressed event journal with cryptographic chain integrity.
That gives reviewers a concrete object to inspect instead of reconstructing behavior from partial logs.

At a high level:

- Events are linked in order and integrity-checked.
- Session contents can be replayed deterministically against recorded providers/tools.
- Two sessions can be compared structurally and at field level.
- Evidence can be exported into AGEF bundles for portability.

Akmon v2.0.0 implements AGEF v0.1.1 as a practical reference implementation for portable AI-agent session evidence.
The goal is operational interoperability and verifiability, not vendor-specific lock-in.

### The 10-command surface in v2.0.0

v2.0.0 ships ten commands organized around the session lifecycle.

#### Run and diagnose

- `run` starts normal agent execution.
- `doctor` validates environment and configuration preconditions.

#### Inspect and understand sessions

- `inspect` reads session records for targeted examination.
- `diff` compares sessions structurally and at field level.

#### Replay and verify integrity

- `replay` re-executes against recorded provider/tool context for deterministic validation.
- `verify` performs integrity checks on session artifacts.
- `audit` validates cryptographic chain integrity and audit consistency.

#### Create compliance artifacts

- `evidence` generates evidence outputs suitable for review workflows.
- `bundle` packages portable AGEF archives.
- `redact` removes sensitive content from artifacts under controlled policy.

These commands are meant to compose.
A common pattern is:

1. run a task,
2. inspect/diff output,
3. verify/audit integrity,
4. export/redact evidence for downstream review.

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
- Session comparison: [Session diff reference](./reference/diff.md)
- Architecture for contributors: [Contributing architecture](./contributing/architecture.md)
