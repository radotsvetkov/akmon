# Contributing to Akmon

Thanks for your interest in improving Akmon. Akmon does one job: it seals AI agent sessions into signed, tamper-evident evidence that someone who doesn't trust you can verify offline. Contributions that make that job more correct, more honest or easier are the most welcome.

All participants must follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report security issues privately as described in [SECURITY.md](SECURITY.md). Never open public issues for undisclosed vulnerabilities.

## Developer Certificate of Origin (DCO)

Akmon uses the [Developer Certificate of Origin](https://developercertificate.org). By signing off a commit, you certify that you wrote it or otherwise have the right to submit it under the project's license (Apache-2.0).

Add a `Signed-off-by` line to every commit, which Git does for you with `-s`:

```bash
git commit -s -m "fix(bundle): reject duplicate manifest entries"
# -> Signed-off-by: Your Name <your.email@example.com>
```

Pull requests with commits that are not signed off will be asked to amend their history before review.

## Ways to contribute

- Report bugs, especially anything where verification output claims more than it proved.
- Improve docs, tutorials and examples (they must be copy-paste runnable).
- Add OpenTelemetry GenAI fixtures from real instrumentations, with provenance.
- Submit focused pull requests with tests.

Open an issue first for large changes, behaviour changes, or anything that touches the AGEF format.

## Local setup

The toolchain is pinned in `rust-toolchain.toml` (Rust **1.88**, the same as CI), so `rustup` selects it automatically.

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon
cargo build --workspace
# binaries: target/debug/akmon and target/debug/agef-verify
```

Optional tools for the full gate set: [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) (`cargo install --locked cargo-deny`), [`mdbook`](https://github.com/rust-lang/mdBook) 0.4.x for the handbook, and OpenSSL 3.x so the end-to-end `openssl` leg runs instead of skipping (macOS ships LibreSSL: `brew install openssl@3`).

## Required gates

CI runs these, so run them before you open a PR:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
RUSTFLAGS='-D warnings' cargo test --workspace --no-default-features
cargo deny check
```

Clippy runs with `--all-targets` because the pinned toolchain also lints test code. Since v3.0 there are no optional cargo features, so `--no-default-features` changes nothing; it stays in the commands for parity with CI.

The end-to-end harness `crates/akmon-cli/tests/e2e_otel_to_openssl_integration.rs` is the frozen proof that a real OTEL trace goes all the way to a stock-OpenSSL verification. Do not weaken it. If it has to change, explain why in the PR.

## Docs checks

When a PR touches `README.md`, `CHANGELOG.md`, `landing/` or `docs/`, run the same deterministic checks as CI (mdbook must be on `PATH`):

```bash
bash scripts/docs/run_all.sh        # mdbook build, links, CLI smoke, JSON snippets, version markers
bash scripts/docs/test_fixtures.sh  # self-tests for the checkers
```

`scripts/docs/check_version_markers.py` requires every `Documented for Akmon \`X.Y.Z\`` marker, the README and the landing page to match the workspace version in `Cargo.toml`. Update them in the same PR that bumps the version.

## Coding standards

- Respect crate boundaries. The trust path (`akmon-journal`, `akmon-bundle`, `agef-verify`, `akmon-otel`) must not depend on the agent crates.
- **Never change AGEF substrate bytes**: event encoding, hashing, signed statements (`AGEF-SIG-v1`, `AGEF-OPERATOR-v1`) or the manifest format. Format changes go through the [AGEF specification](https://github.com/radotsvetkov/agef) first.
- **Fail closed.** A verifier that is unsure must fail. Output must never claim more than was verified (integrity only vs. signature verified, `full` vs. `partial` vs. `structural` capture).
- **Sanitize untrusted text** (anything from a bundle, trace or session) before it reaches a terminal (`akmon_bundle::sanitize_display`, `akmon_diff::sanitize_for_terminal`).
- No `unwrap()`/`expect()` outside tests unless failure is impossible and documented. Slice strings only on character boundaries (`akmon_core::truncate_at_char_boundary`).
- Every public item has a rustdoc comment. User-facing errors say what to do next and use the shared JSON error shape and exit codes (`0` ok, `1` verification failed, `2` usage, `3` I/O).
- Agent file paths always go through sandbox validation. Secrets never appear in logs, errors, journals or argv.

## The `archive/` directory

Code that no longer serves the evidence job lives in [`archive/`](archive/README.md). The workspace excludes it (`exclude = ["archive"]`), so it is **not compiled, tested, linted or shipped**, and `cargo-deny` does not scan it. The complete pre-archive tree is tagged `archive/pre-focus-v2.2.1`.

- Don't fix or modernize archived code, and don't add new code there except by moving it with `git mv` (which keeps history).
- To bring an item back, open an issue explaining how it serves the evidence job. Then restore it with `git checkout archive/pre-focus-v2.2.1 -- <path>`, re-add its wiring, give it tests, and update `archive/README.md` and the CHANGELOG.
- Retired handbook pages go to `archive/docs/`.

## Commits and pull requests

- Use Conventional Commit style subjects (`fix(bundle): …`, `feat(cli): …`, `docs: …`) and explain *why* in the body.
- Keep each PR to one logical change, without unrelated refactors.
- PR description: problem, approach, trade-offs, test plan, and any user-visible change (CLI, JSON, exit codes, docs).
- User-visible changes need a `CHANGELOG.md` entry under `[Unreleased]` and the matching README or `docs/` update.

## Security- and policy-sensitive changes

For changes to verification, signing, archive parsing, sandboxing, permissions, shell, MCP or network access:

- explain the threat model in the PR;
- add regression tests for both the accept and the reject paths;
- call out any new permission surface or any change in what a verdict means.

## Questions

If you're unsure about direction, open a draft PR early or start a discussion in an issue.
