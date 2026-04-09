# `AKMON.md` guide

`AKMON.md` is the highest-leverage file in an Akmon project. It is loaded at session start and continuously influences planning, tool selection, and verification behavior.

## Why `AKMON.md` matters more than a one-off prompt

Prompts are ephemeral. `AKMON.md` is persistent and reused every run. If you encode architecture boundaries and verification commands in this file, the agent can follow them automatically across turns and sessions.

Examples:

- if `AKMON.md` says `verify: cargo check 2>&1 | head -20`, the agent tends to run that after edits,
- if it says `repository pattern only`, the agent is less likely to generate active-record style shortcuts,
- if it says `no unwrap() in library crates`, reviews and fixes stay aligned.

## Anatomy of an effective `AKMON.md`

Use concise sections:

- **Product:** what this project does and key constraints,
- **Architecture:** module boundaries and forbidden dependencies,
- **Conventions:** code style, error handling, naming, test policy,
- **Verification:** canonical commands per change type,
- **Current sprint:** immediate goals and priorities.

### Example: Rust service

```markdown
# Payment Service

Stripe payment processing microservice.
Rust 1.75 + Axum 0.7 + PostgreSQL + SQLx.

## Architecture
- domain/: pure business logic
- ports/: trait interfaces
- adapters/: db/http/stripe implementations
- application/: orchestration layer

Never import adapters into domain.

## Error handling
Use thiserror in domain, anyhow in orchestration.

## Verification
After Rust file: cargo check 2>&1 | head -20
After business logic: cargo test domain 2>&1
After handlers: cargo test integration 2>&1
```

### Example: Python FastAPI

```markdown
# User Analytics API

FastAPI + PostgreSQL + Redis event tracking.

## Layout
src/api/routes/
src/services/
src/repositories/
src/models/
src/schemas/

## Conventions
- routes -> services -> repositories
- no direct repository calls from routes
- strict schema validation

## Verification
After Python file: python -m py_compile {file}
After models: alembic check
After routes: pytest tests/api/ -x -q
```

## The 2000-character rule

`AKMON.md` appears in many model calls. Oversized context inflates recurring input tokens and reduces room for live task reasoning.

Practical guideline:

- target <= 2000 characters,
- keep durable details in `AKMON.md`,
- move long implementation plans into `.akmon/specs/*.md`.

## Maintenance workflow

1. initialize or refresh with `akmon init`,
2. edit manually or via `/update-context`,
3. review after major architecture changes,
4. keep `Current sprint` up to date.

## Common mistakes and troubleshooting

- **Too vague:** "clean code, best practices" is not actionable.
- **Too long:** giant prose blocks are expensive and low signal.
- **Missing verification commands:** agent cannot infer your CI expectations reliably.
- **Stale sprint section:** leads to drift and irrelevant actions.
