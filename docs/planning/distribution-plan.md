# Distribution plan (DRAFT — requires human approval before any irreversible step)

**Status:** planning / not executed. **Date:** 2026-06-06.
**Owner decision required:** every step that publishes under the project's identity or to a public
namespace is a **one-way door** and must be approved by the maintainer. Nothing here is done
autonomously.

## Goal

Make the auditor-facing verifier (`agef-verify`) and the `akmon` CLI **trivially obtainable and
trustworthy to install**. For a *trust* product, the install path must itself be verifiable — a bare
`curl | sh` with no checksum or signature is an own-goal.

## What works today (no new infrastructure)

- **From source, any platform:**
  - `cargo install --git https://github.com/radotsvetkov/akmon akmon`
  - `cargo install --git https://github.com/radotsvetkov/akmon agef-verify`
- **Prebuilt binary:** a GitHub release exists with at least `akmon-linux-x86_64`
  (`releases/latest/download/...`). The README documents both paths.

These are the install instructions the README ships **now**. Everything below is incremental.

## Phase D-1 — Release workflow + checksums (low risk, reversible)

A GitHub Actions workflow, triggered on a version tag, that:
1. Cross-compiles `akmon` and `agef-verify` for the major targets
   (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`,
   `x86_64-pc-windows-msvc`).
2. Publishes each artifact **with a `SHA-256SUMS` file** attached to the release.
3. (Optional, recommended) signs the `SHA-256SUMS` with the project's release key, and documents the
   public key so users verify the download — eating our own dog food (the whole product is "verify
   before you trust").

Reversible: a bad release can be deleted/yanked; tags can be moved before announcement.

## Phase D-2 — Homebrew tap (low risk)

A `radotsvetkov/homebrew-akmon` tap with a formula that pulls the D-1 release tarball **and verifies
its SHA-256**. Cask/formula updates are reversible. Requires D-1 (needs a release + checksums first).

## Phase D-3 — crates.io publish (HIGH CARE — one-way door, USER-GATED)

Publishing to crates.io is a **SemVer + namespace commitment that cannot be undone** (yanks remain
visible forever) and prematurely freezes public API surfaces. Per decision **D-15**, do not publish
until the maintainer explicitly approves. Specific complications to resolve first:

- The workspace is ~13 crates. crates.io forbids `path`-only inter-crate deps, so every published
  crate needs a real version requirement on its published dependencies, and publishing must happen in
  dependency order (leaf crates first).
- Every published crate needs complete metadata: `description`, `license`, `repository`,
  `rust-version`, `keywords`, `categories`, `readme`.
- Decide the **public surface**: likely only `akmon` (CLI) and `agef-verify` are meant for end users;
  the library crates may be published as supporting deps or kept internal (`publish = false`).
- Naming: confirm the `akmon` / `agef-verify` crate names are available and acceptable as a permanent
  claim.

**Recommendation:** ship D-1 + D-2 first (they cover real adoption needs and are reversible). Defer
D-3 until the public API is stable — in particular until the operator-identity work lands, since that
may add CLI surface that we would not want to freeze prematurely.

## Security constraints (apply to all phases)

- The download path for a trust tool must be verifiable: publish checksums, and sign them.
- Never recommend an unverified `curl | sh`.
- Keep release-signing keys out of the repo and out of CI logs; treat them with the same hygiene the
  product preaches for signing keys (`akmon bundle keygen` writes private keys `0600`).
