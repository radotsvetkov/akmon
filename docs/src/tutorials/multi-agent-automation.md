# Multi-agent automation in practice

Multi-agent in Akmon means decomposing work into multiple focused sessions instead of one giant context-heavy loop.

## The problem with single-agent sessions

If you ask one session to both explore and implement a large subsystem, context fills with stale file reads and old reasoning. By the time code generation starts, the model is paying token/cognitive budget for irrelevant history.

Typical failure pattern:

1. reads 10-20 files,
2. repeats exploration because context is noisy,
3. implementation quality drops,
4. token cost rises.

## How `spawn_subagent` helps

A subagent run is a fresh focused session. It performs exploration and returns a compact summary to the main session. The main implementation session does not carry every exploratory file read in its context.

Net effect:

- smaller implementation context,
- fewer repeated reads,
- clearer plan/spec handoff,
- lower token waste.

## Safety defaults for nested runs

Subagents are intentionally conservative:

- no implicit broad approvals are injected at nested bootstrap,
- nested execution is capped by the parent permission posture,
- when parent policy is interactive, nested runs stay read-oriented unless policy can be
  satisfied automatically,
- side-effect tools that exceed the parent ceiling are filtered out before nested execution.

Practical guidance:

- use subagents for research/summarization,
- perform high-impact write/shell steps in the main session where approvals are explicit,
- if a nested run reports ceiling restrictions, tighten task scope to read-only discovery.

## Three-phase pattern

### Phase 1: Research (subagent)

Goal: understand codebase boundaries and constraints.

Prompt:

```text
Explore authentication flow and summarize entry points, middleware, data models, and tests. Return only structured findings.
```

### Phase 2: Specification (main session)

Goal: persist plan to disk.

```bash
akmon --plan --task "Write implementation plan for OAuth integration using research summary"
```

Creates `.akmon/specs` or plan artifacts that survive compaction/restart.

### Phase 3: Implementation (main session)

Goal: execute one checked step at a time with verification.

Prompt style:

```text
Implement step 1 only, run verification commands, then stop.
```

Repeat for each step.

## Real example: adding a payment system

Research prompt:

```text
Find existing payment-related code, billing models, and webhook endpoints. Summarize what exists and what is missing.
```

Plan file example:

```markdown
# Plan: Stripe Payment Integration

## Research findings
- Current payment code: none
- User model in src/models/user.rs has email field
- API is Axum with JWT auth in src/middleware/auth.rs

## Implementation steps
- [ ] Add stripe dependency to Cargo.toml
- [ ] Create src/payments/mod.rs with Stripe client setup
- [ ] Create src/payments/checkout.rs with create_session
- [ ] Create src/payments/webhook.rs with event handling
- [ ] Add POST /payments/checkout route
- [ ] Add POST /payments/webhook route
- [ ] Add payment_status to user model
- [ ] Write integration tests
```

Implementation run:

```text
Implement the next unchecked payment step. Run tests relevant to touched files.
```

## Parallel research strategy for large monorepos

For very large repositories:

1. run multiple research tasks by domain (auth, billing, API, infra),
2. produce short summaries per domain,
3. merge into one implementation plan.

This is more reliable than one massive exploratory session.

## Common mistakes and troubleshooting

- **Skipping written plan:** always persist to spec/plan before implementation.
- **Research summary too verbose:** ask for bullet-point outputs with file paths only.
- **Main session still bloated:** reset implementation session and re-run from plan.
- **Unclear ownership in automation:** assign per-phase prompts and expected outputs explicitly.
