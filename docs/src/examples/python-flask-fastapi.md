# Python web services — Flask & FastAPI

Documented for Akmon `2.0.0`.

## Scenario

Industry/context (illustrative): healthcare integration service requiring auditable API modifications.

## Constraints

- Approval requirement: auth and validation behavior must follow existing conventions.
- CI requirement: `pytest` required for each change slice.
- Audit need: evidence artifact retained for API behavior review.

This example complements the [Python language guide](../languages/python.md) and the [step-by-step tutorial](../tutorials/step-by-step.md).

## What you will demonstrate

- A **read-only plan** for a small API change (`akmon --plan`).
- A **single headless implementation pass** (`akmon --yes`) with tests.
- **AKMON.md** steering for framework-specific layout (app factory vs `main.py`, router modules, etc.).

## Flask — readiness endpoint

**Context in `AKMON.md`:** Document how the app is created (`create_app`), where config lives, and the test command (`pytest`).

```bash
akmon --plan --task "Add GET /api/ready that returns {status: ok} and optionally verifies DB with existing engine"
akmon --yes --task "Implement /api/ready per the latest plan; use application factory pattern already in repo"
pytest
```

## FastAPI — authenticated route

**Context:** List dependencies used for auth (e.g. `OAuth2PasswordBearer`, custom `get_current_user`).

```bash
akmon --plan --task "Add GET /users/me using existing JWT dependency; specify files to edit"
akmon --yes --task "Implement /users/me; mirror error handling from similar routes; add async tests"
pytest
```

## Tips

- Put **one** test command in **Conventions** so every session agrees on verification.
- If you use **OpenAPI**, mention where the spec is generated—helps the agent avoid duplicate route definitions.
- For long handlers, prefer **plan mode** first so file boundaries are explicit before writes.

See also: [Headless mode](../usage/headless.md), [Security model](../features/security.md).

## Outcome

This flow provides framework-specific modifications with explicit test and evidence expectations.

## Anti-patterns

- Adding authenticated routes without mirroring existing auth dependency patterns.
- Accepting framework drift (mixed app-factory and module-level startup styles) in the same refactor.
