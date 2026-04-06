# Rust Projects

Akmon detects Rust projects from **`Cargo.toml`** and applies Rust-oriented conventions and framework hints via the project intelligence layer.

## Auto-detection

When Akmon finds `Cargo.toml`:

- Language profile **Rust**
- Workspace members when present
- Framework hints from dependencies (`axum`, `actix-web`, `tokio`, `sqlx`, `diesel`, `ratatui`, `clap`, `tauri`, `bevy`, …)

## Conventions (steering)

Typical guidance injected for Rust codebases:

- `thiserror` for library errors, `anyhow` for application binaries (where appropriate)
- Avoid `.unwrap()` in production paths
- Prefer borrowing over unnecessary clones
- Document public items (`rustdoc`)
- Use `spawn_blocking` for CPU-heavy work inside async runtimes

Framework-specific notes (e.g. Axum handlers thin → services; SQLx `query!` and pools) are added when dependencies match.

## Example: plan an Axum API

```bash
cargo new my-api && cd my-api
# add axum, tokio, sqlx, serde, thiserror, anyhow …
akmon --plan \
  --task "build a REST API with user authentication,
  PostgreSQL via SQLx, JWT tokens,
  layered architecture (handler → service → repository),
  and proper error handling"
```

Then implement when satisfied with the plan.

## Example: explore a workspace

```bash
cd my-workspace
akmon chat
```

```
explain how akmon-core relates to akmon-tools
and what the data flow is between crates
```

## Common Rust tasks

| Task | Prompt |
|---|---|
| Error handling | `replace unwrap() calls with proper Result handling` |
| Testing | `add unit tests for the authentication module` |
| Documentation | `add rustdoc to all public items in src/lib.rs` |
| Clippy | `fix all clippy warnings in the workspace` |

See [Semantic search](../features/semantic-search.md) for `--index` usage on large Rust trees.
