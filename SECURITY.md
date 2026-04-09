# Security policy

**Do not** open public issues for **undisclosed** security vulnerabilities. Use [GitHub private vulnerability reporting](https://github.com/radotsvetkov/akmon/security/advisories/new) when available, or contact the maintainer privately (see the repository owner’s GitHub profile).

Include:

- Description and impact
- Reproduction steps
- Affected versions / commits if known
- Optional patch ideas

**Target initial response:** 48 hours (best effort).

## Scope

**In scope**

- Sandbox bypass or path traversal outside the repository root
- SSRF bypasses in `web_fetch`
- Secret leakage via logs, errors, or persistence
- Permission / policy bypass leading to silent destructive actions

**Out of scope**

- Physical access scenarios
- Social engineering
- Issues solely inside third-party dependencies (report upstream)

## More documentation

- Runtime security model: [docs/src/features/security.md](docs/src/features/security.md)
- Full book (hosted): [Security](https://radotsvetkov.github.io/akmon/docs/security.html) and [Security model](https://radotsvetkov.github.io/akmon/docs/features/security.html)
