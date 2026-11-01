# Spec Workflow

For building new features from scratch with structured planning.
Three phases: requirements, then design, then tasks, then implementation.

## Overview

```bash
# Phase 1: Generate requirements
akmon spec auth-system "JWT authentication with refresh tokens"

# Phase 2: Generate technical design (after reviewing requirements)
akmon spec auth-system design

# Phase 3: Generate implementation tasks
akmon spec auth-system tasks

# Implement one task at a time
akmon spec auth-system implement
```

Artifacts live under `.akmon/specs/<name>/`.

## Phase 1: Requirements

```bash
akmon spec payment-flow \
  "Stripe payment integration with webhook handling \
   and subscription management"
```

Produces `.akmon/specs/payment-flow/requirements.md` with user stories, acceptance criteria, scope, and open questions.

## Phase 2: Design

```bash
akmon spec payment-flow design
```

Reads `requirements.md`, analyzes the codebase, and writes `design.md` with architecture, new components, modified files, and data flow.

## Phase 3: Tasks

```bash
akmon spec payment-flow tasks
```

Writes `tasks.md` with checkboxes, dependencies, and sized work items.

## Implementation

```bash
akmon spec payment-flow implement
```

Akmon picks the first unchecked task, implements it, checks it off, and stops for human review. Re-run for the next task.

This **human-in-the-loop per task** pattern limits runaway changes that drift from the spec.

## See also

- [CLI reference](../reference/cli.md) for exact `akmon spec` syntax and flags.
