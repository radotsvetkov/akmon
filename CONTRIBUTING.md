# Contributing to Akmon

Thanks for your interest in improving Akmon.

This guide explains how to contribute code, docs, and bug reports in a way that is easy to review and ship.

All participants must follow the **[Code of Conduct](CODE_OF_CONDUCT.md)**. Report **security** issues per **[SECURITY.md](SECURITY.md)** (do not file public issues for undisclosed vulnerabilities).

## Ways to contribute

- Report bugs and UX issues.
- Propose features and architecture improvements.
- Improve docs, tutorials, and examples.
- Submit focused pull requests with tests.

## Before you start

- Search existing issues and pull requests to avoid duplicates.
- Open an issue first for large changes or behavior changes.
- Keep one pull request scoped to one logical change.

## Local setup

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon
cargo build --release
```

Optional smaller build (without semantic indexing):

```bash
cargo build --release --no-default-features
```

## Development workflow

1. Create a branch from `main`.
2. Implement the change.
3. Add or update tests.
4. Run checks locally.
5. Open a pull request with context and test notes.

## Required checks

Run these before opening a PR:

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

If you touched no-default-features behavior, also run:

```bash
cargo build --release --no-default-features
```

## Docs checks

When your PR touches documentation, run the same deterministic docs checks used by CI:

```bash
# Prerequisite: mdbook installed and available on PATH
bash scripts/docs/run_all.sh
```

You can also run checks individually:

```bash
mdbook build docs
python3 scripts/docs/check_markdown_links.py
bash scripts/docs/check_cli_smoke.sh
python3 scripts/docs/check_json_snippets.py
bash scripts/docs/test_fixtures.sh
```

## Coding standards

- Match existing crate boundaries and style.
- Prefer small, composable functions.
- Avoid unnecessary abstractions.
- Add rustdoc to new public APIs.
- Do not use `unwrap()` in library code unless failure is truly impossible and documented.
- Keep user-facing errors actionable.

For TUI changes, place state-transition logic in focused `crates/akmon-tui/src/state/*` modules and keep `TuiApp` as orchestration/composition glue.

## Commit and PR guidelines

- Use clear commit messages that explain why.
- Keep diffs focused; avoid unrelated refactors.
- Include a concise PR description:
  - Problem
  - Approach
  - Trade-offs (if any)
  - Test plan

## Documentation changes

- Update `README.md` when behavior visible to users changes.
- Update `docs/` and `CHANGELOG.md` for release-relevant changes.
- Prefer examples that can be copied and run as-is.

## Security and policy-sensitive changes

Akmon prioritizes trust and auditability.

For changes around sandboxing, permissions, shell, MCP, or network access:

- explain the risk model in the PR,
- include regression tests,
- and call out any new permission surface explicitly.

## Questions

If you are unsure about direction, open a draft PR early or start a discussion in an issue.

