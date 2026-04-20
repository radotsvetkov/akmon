# Step-by-step: real workflows from zero to shipped

This page walks through four complete tutorials with concrete commands, expected outputs, common failures, and resulting files.

## Tutorial 1: Rust + Axum REST API from scratch

### Setup

```bash
mkdir -p ~/projects/bookshelf-api
cd ~/projects/bookshelf-api
git init
export ANTHROPIC_API_KEY=sk-ant-...
akmon chat
```

Create `AKMON.md` (via `/init` or manually):

```markdown
# Bookshelf API

Rust + Axum + SQLite REST API for managing books.

## Stack
- Language: Rust 1.75+
- Framework: Axum 0.7
- Database: SQLite via rusqlite
- Auth: JWT with jsonwebtoken

## Conventions
- Error type: AppError implementing IntoResponse
- Database: connection pool via r2d2
- Verify: cargo check 2>&1 | head -20
```

### Session flow

Prompts to send in order:

1. `Initialize the project with Cargo.toml and basic dependencies`
2. `Create src/main.rs with Axum app bootstrap and health endpoint`
3. `Create src/models/book.rs with CRUD model operations`
4. `Create src/routes/books.rs with GET /books and POST /books`
5. `Wire routes into main and add minimal integration tests`

What you should see:

- tool calls to `write_file` for `Cargo.toml` and `src/*`,
- permission dialog per write (press `y` once or `s` for session allowance),
- verification shell commands (`cargo check`) after write batches,
- final `Done` plus cost/token summary.

Expected output tree:

```text
bookshelf-api/
  Cargo.toml
  src/
    main.rs
    error.rs
    routes/
      mod.rs
      books.rs
    models/
      mod.rs
      book.rs
  tests/
    books_api.rs
```

### If something goes wrong

- **`cargo check` fails:** ask `Fix compile errors only, no refactor`.
- **Agent loops on reads:** ask `Stop exploration and implement from current context`.
- **Rate limited:** run `akmon -c` to continue.

## Tutorial 2: Python FastAPI + PostgreSQL

### Setup

```bash
mkdir -p ~/projects/users-api
cd ~/projects/users-api
git init
python -m venv .venv
source .venv/bin/activate
akmon chat
```

Use explicit 3-phase flow:

1. **Research:** `Explore this repo and propose FastAPI + SQLAlchemy layout`
2. **Plan:** `/plan` then `Write a step-by-step implementation plan`
3. **Implement:** `/implement`

Prompt examples:

- `Create pyproject.toml, app entrypoint, and dependency set`
- `Add SQLAlchemy models for users table and repository layer`
- `Add FastAPI routers for GET /users and POST /users`
- `Add pytest tests for validation and database behavior`

Expected files:

```text
users-api/
  pyproject.toml
  app/
    main.py
    db.py
    models.py
    schemas.py
    repository.py
    routes/users.py
  tests/
    test_users.py
```

Troubleshooting:

- **DB connection error:** provide a local `DATABASE_URL` in `.env`.
- **pytest import errors:** ask agent to fix Python path/package init files only.

## Tutorial 3: TypeScript/Next.js full-stack app

### Setup

```bash
mkdir -p ~/projects/notes-web
cd ~/projects/notes-web
git init
akmon chat --model anthropic/claude-haiku-4-5
```

Use architect mode for split reasoning:

```bash
akmon --architect \
  --planner-model llama3.2 \
  --model anthropic/claude-haiku-4-5 \
  --task "Create a Next.js notes app with API routes and sqlite persistence"
```

What this demonstrates:

- planner creates architecture first,
- implementer executes files in focused steps,
- context remains cleaner than one long free-form run.

Expected files:

```text
notes-web/
  package.json
  app/
    page.tsx
    api/notes/route.ts
  lib/
    db.ts
    notes.ts
  tests/
    notes.test.ts
```

Troubleshooting:

- **Type errors:** ask `Run tsc and fix only reported errors`.
- **Next route mismatch:** ask `Align route handler signatures with Next version in package.json`.

## Tutorial 4: Refactoring an existing codebase

### Setup

```bash
cd ~/projects/existing-service
akmon --plan --task "Analyze auth module and propose OAuth migration plan"
```

Then:

1. review generated plan,
2. run implementation in focused steps,
3. continue with `akmon -c` if interrupted/rate-limited.

Recommended prompts:

- `Implement step 1 only from the plan; run tests`
- `Implement next unchecked step and verify`
- `Summarize changed files and remaining plan items`

Audit review:

```bash
ls .akmon/audit/
jq . .akmon/audit/<latest>.jsonl | head -40
```

What you should see:

- policy decisions per write/shell call,
- tool outputs tied to each step,
- clear trail of refactor sequence.

## Common mistakes

- Asking for "build everything" in one turn.
- Missing `AKMON.md` conventions (verification commands, architecture boundaries).
- Running in headless mode without budget limits.

## Next steps

- Multi-agent patterns: [multi-agent automation](./multi-agent-automation.md)
- Headless CI workflows: [headless mode](../usage/headless.md)
- Project context quality: [AKMON.md guide](../project/akmon-md.md)

## Scout dossier to implementation

Use the bounded scout flow to improve implementation context without enabling broad multi-agent orchestration:

```bash
# 1) Generate a read-only dossier
akmon scout \
  --task "find provider resolution paths and doctor coverage gaps" \
  --max-files 250 \
  --out .akmon/context/provider-scout.json

# 2) Inspect the dossier
jq '.confidence, .candidate_files[0:5], .constraints' .akmon/context/provider-scout.json

# 3) Run implementation with dossier context injected
akmon \
  --dossier .akmon/context/provider-scout.json \
  --task "implement provider resolution explainability with tests and docs"
```
