# Step-by-step: first hour with Akmon

Every path below follows the same rhythm so you can compare stacks:

1. **Enter the repo** and run `akmon init` (or create `AKMON.md` by hand).
2. **Plan only** — `akmon --plan --task "…"` — verify `.akmon/plans/` output.
3. **Implement once** — `akmon --yes --task "…"` or interactive `akmon chat`.
4. **Inspect audit** — `ls .akmon/audit/` and skim the latest JSONL.

Use `--output json` with `--yes` when you need machine-readable results for scripts ([Headless mode](../usage/headless.md)).

---

## Rust (workspace or single crate)

```bash
cd my-rust-service
akmon init
```

In `AKMON.md`, fill **Current sprint** with one sentence (e.g. “Add idempotency keys to the payment command”).

**Plan:**

```bash
akmon --plan --task "List files to touch for idempotency keys in the payment flow and outline test commands"
```

**Implement (headless):**

```bash
akmon --yes --task "Implement idempotency key storage using the project's existing DB layer; add tests"
```

**Verify:**

```bash
cargo test
```

**Why this works:** Akmon’s project intelligence injects Rust-oriented hints (see [Rust projects](../languages/rust.md)) so the model reaches for `cargo test` / `cargo clippy`-style checks without you repeating them every prompt.

---

## Go (module under `go.mod`)

```bash
cd my-go-api
akmon init
```

**Plan:**

```bash
akmon --plan --task "Propose package layout for a new /v2 health handler with structured logging"
```

**Implement:**

```bash
akmon --yes --task "Add GET /v2/health returning JSON; use existing logger; add table-driven test"
```

**Verify:**

```bash
go test ./...
```

See [Go projects](../languages/go.md) for conventions the agent tends to follow once `AKMON.md` names your module boundaries.

---

## Python — Flask

```bash
cd my-flask-app
akmon init
```

Describe in `AKMON.md` how you run the app (e.g. `flask run` or `gunicorn`).

**Plan:**

```bash
akmon --plan --task "Plan adding a /api/ready endpoint that checks DB connectivity without exposing secrets"
```

**Implement:**

```bash
akmon --yes --task "Implement /api/ready in the existing Flask app factory; add a small test using the project's test client"
```

**Verify:**

```bash
pytest   # or python -m pytest, matching your repo
```

---

## Python — FastAPI

```bash
cd my-fastapi-service
akmon init
```

**Plan:**

```bash
akmon --plan --task "Sketch router changes for JWT-protected /users/me using existing auth dependencies"
```

**Implement:**

```bash
akmon --yes --task "Add GET /users/me with existing JWT dependency; return 401 when missing; add async test"
```

**Verify:**

```bash
pytest
```

Tip: put **one** canonical test command in `AKMON.md` so plan and implement steps stay consistent across sessions.

---

## Elixir (Mix / Phoenix)

```bash
cd my_phoenix_app
akmon init
```

Mention `mix` tasks you use (`mix test`, `mix format`, Dialyzer if applicable) in **Conventions**.

**Plan:**

```bash
akmon --plan --task "Plan a LiveView or controller change for an admin-only settings page; list contexts to touch"
```

**Implement:**

```bash
akmon --yes --task "Implement the settings page per plan; add ExUnit tests for the context"
```

**Verify:**

```bash
mix test
```

Akmon still benefits even when BEAM conventions are niche: explicit **Architecture** and **Conventions** sections in `AKMON.md` reduce hallucinated module names.

---

## Next steps

- Automate the same flow in CI: [Multi-agent & automation](./multi-agent-automation.md)
- Scale planning across roles (cheap planner model + strong implementer): [Architecture patterns](./architecture-patterns.md)
- Deep dive: [Examples](../examples/rust-axum-api.md), [Spec workflow](../usage/spec-workflow.md)
