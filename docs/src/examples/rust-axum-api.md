# Example: REST API with Rust + Axum

Documented for Akmon `2.1.0`.

End-to-end pattern: **plan, scaffold, implement in layers, then test**.

## Scenario

Industry/context (illustrative): defense-adjacent internal API with strict review and evidence expectations.

## Constraints

- Data boundary: local/dev database only for example.
- Approval requirement: layered implementation and test coverage.
- CI requirement: reproducible `cargo` checks.

## Prerequisites

- Rust toolchain (see [Installation](../getting-started/installation.md))
- PostgreSQL (local or Docker)
- Provider API key or Ollama

## Scaffold

```bash
cargo new blog-api && cd blog-api
```

Add dependencies (`axum`, `tokio`, `sqlx`, `serde`, `thiserror`, `tower-http`, `tracing`, auth crates, …) to `Cargo.toml`.

## Plan

```bash
akmon --plan --task "blog REST API: register/login, JWT middleware,
CRUD posts, SQLx + Postgres pool, layered handlers,
custom ApiError as IntoResponse, GET /health"
```

Review `.akmon/plans/*.md` before implementation.

## Implement incrementally

Typical sequence:

1. `error.rs`: error types + `IntoResponse`
2. `state.rs`: shared `PgPool`
3. Repositories / services
4. Routes + integration tests (`sqlx::test`)

```bash
akmon --yes --task "implement error + state modules from the plan"
akmon --task "implement repositories and handlers per plan"
```

## Run

```bash
docker run -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
export DATABASE_URL=postgres://postgres:postgres@localhost/blog_api
sqlx database create && sqlx migrate run
cargo run
```

## Follow-ups

```
add cursor-based pagination to GET /posts
```

```
add utoipa OpenAPI spec and Swagger UI
```

## Outcome

Artifacts answer: "What changed, why, and can we verify the session chain?"
- `.akmon/audit/<session-id>.jsonl`
- `.akmon/evidence/<session-id>.json`
- Optional headless `run.json` for CI records

## Anti-patterns

- Implementing auth + persistence + routing in a single unreviewed turn.
- Skipping `--plan` for large multi-module changes.
