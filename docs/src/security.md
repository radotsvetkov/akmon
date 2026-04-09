# Security Policy

The same reporting rules and scope are maintained in the repository root as [`SECURITY.md`](https://github.com/radotsvetkov/akmon/blob/main/SECURITY.md) for GitHub’s security features.

## Reporting vulnerabilities

**Do not** open public issues for undisclosed security problems.

Contact the maintainer privately (see the GitHub profile / repository security instructions). Include:

- Description and impact
- Reproduction steps
- Affected versions / commits if known
- Optional patch ideas

Target initial response: **48 hours** (best effort).

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

## Design reference

Read [Security model](./features/security.md) for how Akmon is intended to behave at runtime.
