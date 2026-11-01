# Security policy

Akmon is a trust tool: people rely on `agef-verify` and `akmon bundle verify` to tell them whether evidence is genuine. We treat a verification bypass as the most serious kind of bug, and we would much rather hear about one privately than read about it.

## Supported versions

| Version | Status | Security fixes |
| --- | --- | --- |
| 3.0.x | Current | Yes |
| 2.2.x | Previous | Security fixes only, until **2027-05-01** |
| < 2.2 | Unsupported | No. Please upgrade. |

Bundles are forward-compatible: AGEF v0.1.x bundles produced by 2.x verify with 3.0.x, so upgrading the verifier is always safe.

## Reporting a vulnerability

**Do not open a public issue, discussion or pull request for an undisclosed vulnerability.**

- Preferred: [GitHub private vulnerability reporting](https://github.com/radotsvetkov/akmon/security/advisories/new).
- Alternative: contact the maintainer privately through the repository owner's GitHub profile and ask for a private channel.

Please include:

- affected version(s) or commit, and platform;
- a description of the impact (what an attacker gains);
- minimal reproduction steps or a proof of concept (a crafted `.akmon` bundle, trace or key file is ideal);
- any suggested fix, and whether you want to be credited.

## Coordinated disclosure process

| Step | Target |
| --- | --- |
| Acknowledge your report | within **3 business days** |
| Initial assessment (validity, severity, affected versions) | within **10 business days** |
| Fix or mitigation for critical / high severity | within **30 days** of confirmation |
| Fix for medium / low severity | in the next scheduled release, at most **90 days** |
| Public advisory (GitHub Security Advisory, CVE when applicable) | when the fix is released, or at **90 days** after the report, whichever comes first, unless we agree a different date with you |

We keep you updated at each step, credit you in the advisory and the CHANGELOG unless you prefer otherwise, and ask that you give us the time above before publishing. If a vulnerability is being actively exploited, we may publish an advisory with mitigations before a fix is available. These targets are best effort for a small maintainer team.

## Scope

**In scope: the trust layer** (`akmon-bundle`, `akmon-journal`, `akmon-otel`, `agef-verify`, `akmon seal` and the `akmon bundle` commands):

- **Verification bypass:** a tampered, truncated, reordered or re-signed bundle that `agef-verify` or `akmon bundle verify` reports as passing, or that `--require-signature`, `--require-operator-key` or `--require-capture full` fails to reject.
- **Signature forgery or confusion:** producing an `AGEF-SIG-v1` or `AGEF-OPERATOR-v1` signature that verifies without the private key, or cross-protocol or domain-separation confusion between statements.
- **Misleading verdicts:** output claiming more than was verified, for example "signature verified" for a bundle whose integrity failed, or a capture level shown higher than the one recorded.
- **Archive parsing:** memory exhaustion (decompression bombs past `--max-decoded-bytes`), path traversal, symlink or hardlink tricks, duplicate-entry confusion, or panics on crafted `.akmon` files, traces or key files.
- **Terminal injection:** escape sequences or bidi controls from a bundle, trace or session reaching the terminal unsanitized through any `akmon` or `agef-verify` output.
- **`prove-openssl` artifacts** that let stock OpenSSL "verify" a bundle that Akmon itself would reject.
- **Key handling:** private-key material written with loose permissions, printed, logged or left behind.

**In scope: the bundled reference agent** (`akmon-core`, `akmon-tools`, `akmon-query`, `akmon-models`, `akmon-tui`, `akmon-config`):

- sandbox escapes or path traversal outside the project root, including through symlinks or temp files;
- policy or permission bypass (a denied read, write, shell, network or MCP action that runs anyway);
- SSRF bypasses in `web_fetch`;
- secret leakage through logs, errors, journals, bundles, argv or files with loose permissions.

**Out of scope**

- Attacks that require the signer's private key, or control of the channel you use to distribute the public key. Akmon documents that key trust is established out of band.
- The *truth* of self-asserted operator identity strings. Only the operator key is verified, by design.
- Missing trusted timestamps. This is a documented, known gap (see the README's trusted-time section).
- Prompt-injection outcomes that stay within the permissions the operator granted.
- Physical access, social engineering, and denial of service against your own local machine by your own configuration.
- Vulnerabilities only in third-party dependencies with no Akmon-specific impact. Please report those upstream (we still welcome a heads-up).
- Code under `archive/`, which is not built or shipped.

## Release integrity

Every GitHub release ships `SHA256SUMS` covering all binaries and a CycloneDX SBOM (`akmon.cdx.json`) generated from `Cargo.lock`. Check downloads with `sha256sum --check --ignore-missing SHA256SUMS` (on macOS, `shasum -a 256 --check --ignore-missing SHA256SUMS`). Releases are built only after a gate checks that the tag matches the workspace version and that the workspace tests and clippy pass. Dependencies are checked by `cargo-deny` (RustSec advisories, licenses, sources) on every pull request. Release binaries are not yet signed; signing them is on the roadmap.

## EU Cyber Resilience Act context

Since 11 September 2026, manufacturers of products with digital elements placed on the EU market have had to report actively exploited vulnerabilities and severe incidents under the Cyber Resilience Act (Regulation (EU) 2024/2847, Art. 14). Akmon is free, open-source software developed outside a commercial activity. If you integrate it into a commercial product, those CRA obligations may fall on you as the manufacturer. This policy, the per-release SBOM and the advisory process are meant to help you meet them. If you need a coordinated date for your own CRA reporting, say so in your report and we will work with you.

## More documentation

- Runtime security model of the reference agent: [docs/src/features/security.md](docs/src/features/security.md)
- Trust model and what verification proves: [docs/src/concepts/trust-model.md](docs/src/concepts/trust-model.md)
