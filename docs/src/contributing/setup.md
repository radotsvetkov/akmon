# Development Setup

## Prerequisites

- **Rust** matching `rust-version` in the repo `Cargo.toml`
- **Git**

## Clone

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon
```

## Build

```bash
# Slim / faster — no default feature bundles
cargo build --release --no-default-features

# Full — semantic indexing and related deps
cargo build --release
```

## Test & lint (maintainer expectations)

```bash
RUSTFLAGS='-D warnings' cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Crate map

| Crate | Role |
| --- | --- |
| `akmon-cli` | Binary entry |
| `akmon-core` | Sandbox, policy, FSM, audit |
| `akmon-config` | Configuration |
| `akmon-models` | LLM backends |
| `akmon-tools` | Built-in tools |
| `akmon-query` | Agent session / context |
| `akmon-index` | Semantic index |
| `akmon-tui` | Ratatui front-end |

## Dogfood

```bash
cargo build --release
./target/release/akmon chat
```

## Pull requests

- Clear description + tests where feasible
- No unwrap in library crates
- `rustdoc` on new public APIs
