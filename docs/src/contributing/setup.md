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
cargo build --release -p akmon-cli -p agef-verify
```

## Test & lint (maintainer expectations)

```bash
RUSTFLAGS='-D warnings' cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Crate map

| Crate | Role |
| --- | --- |
| `akmon-journal` | Content-addressed store and hash-linked journal |
| `akmon-bundle` | AGEF bundles: export, import, verify, sign, attest |
| `agef-verify` | Standalone verifier binary |
| `akmon-otel` | OpenTelemetry GenAI importer |
| `akmon-replay` | Deterministic replay |
| `akmon-diff` | Session comparison |
| `akmon-cli` | Binary entry |
| `akmon-core` | Sandbox, policy, FSM, legacy audit chain |
| `akmon-config` | Configuration |
| `akmon-models` | LLM backends (reference agent) |
| `akmon-tools` | Built-in tools (reference agent) |
| `akmon-query` | Agent session / context (reference agent) |
| `akmon-tui` | Ratatui front-end (reference agent) |

Archived code (not built) lives under `archive/`; see `archive/README.md`.

## Dogfood

```bash
cargo build --release
./target/release/akmon chat
```

## Pull requests

- Clear description + tests where feasible
- No unwrap in library crates
- `rustdoc` on new public APIs

Akmon is distributed under the **Apache License 2.0** (see repository `LICENSE`).
