# Security policy

The same reporting rules and scope are maintained in the repository root as [`SECURITY.md`](https://github.com/radotsvetkov/akmon/blob/main/SECURITY.md) for GitHub's security features.

Akmon is an evidence and verification layer. The properties that matter most to its users are integrity, authorship, and offline verifiability of AGEF records. A flaw that lets a tampered record verify as valid, or that lets a signature or operator attestation be forged or bypassed, is the most serious class of issue this project can have. Reports in that area are prioritized accordingly.

## Reporting vulnerabilities

Do not open public issues for undisclosed security problems.

Contact the maintainer privately (see the GitHub profile and the repository security instructions). Include:

- a description and the impact,
- reproduction steps,
- affected versions or commits if known,
- optional patch ideas.

Target initial response: 48 hours, best effort.

## Scope

In scope:

- Verification-layer integrity. Any way to make `akmon bundle verify`, `agef-verify`, or the `openssl` proof path accept a tampered AGEF bundle, a broken hash chain, or a forged or mismatched Ed25519 signature.
- Operator-attestation trust. Any way to make a self-asserted operator identity read as key-verified without a matching trusted key, or to attach a valid attestation to a session it did not authorize.
- Capture-honesty bypass. Any way to make a `structural` import pass `--require-capture full`, or to make a non-replayable session report as replayable.
- Sandbox bypass or path traversal outside the repository root in the reference agent.
- SSRF bypasses in `web_fetch`.
- Secret leakage through logs, errors, or persistence.
- Permission or policy bypass leading to silent destructive actions in the reference agent.

Out of scope:

- Physical access scenarios.
- Social engineering.
- Trusting an attacker-supplied public key. Key trust is established out of band by the verifier; a verifier who chooses to trust a malicious key is outside the model.
- Issues solely inside third-party dependencies (report those upstream).

## Design reference

Read the [Security model](./features/security.md) for how Akmon's verification layer and reference agent are intended to behave, including what trust attaches to keys rather than to self-asserted strings.
