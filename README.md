<div align="center">

<pre>
            ✦        ✦        ✦

           ▓▓▓
           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
         ▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒
         ▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒
           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                   ▓▓▓▓▓▓▓▓▓▓▓▓
                    ▓▓      ▓▓
                    ▓▓      ▓▓
                 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
</pre>

</div>

# Akmon

Akmon is a tamper-evident evidence and verification layer for AI agents. It sits on top of whatever agent you already run, whether that is your own or any tool that emits OpenTelemetry. Each session becomes a portable, content-addressed, cryptographically signed record that someone else can verify for themselves. The part that matters most is this: a third party can check a signature offline using nothing but `openssl`, with no Akmon install, no cloud service, and no need to trust whoever produced the record.

Website: [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/). Documentation: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/).

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![Passed tests](https://img.shields.io/github/actions/workflow/status/radotsvetkov/akmon/ci.yml?branch=main&label=passed%20tests)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## Why this exists

When an AI agent changes something, you may have to prove later what it actually did. The person asking could be a regulator, a security reviewer, or an incident team, and it might be years after the fact. They may not trust you, and they may not run your tools. Most agent telemetry cannot stand up to that. It lives in process memory, or in mutable, unsigned spans, and "the AI did it" is not an answer anyone will accept. Under the EU AI Act, the high-risk logging obligations in Article 12 and Annex IV start to apply on 2 August 2026.

Akmon treats the evidence itself as the thing you ship. It takes a session, either your own or one from any OpenTelemetry-instrumented agent, and turns it into a sealed record. Someone else can then check that record's integrity and authorship independently, with standard tools, even on a machine that has never heard of Akmon.

## The trust toolchain

Every command here ships today and is covered by tests. The focus is the verification chain, not the agent.

| Step | Command | What it does |
| --- | --- | --- |
| Import any agent | `akmon otel import <trace.json>` | Turns an OpenTelemetry GenAI trace into an AGEF session. It reads the current v1.37 structured form and the older v1.36 message-event form that most agents still emit. The capture level (`full` or `structural`) is recorded honestly. |
| Generate a key | `akmon bundle keygen --out k.pk8 --public-out k.pub` | Creates an Ed25519 signing key (PKCS#8 v2). |
| Sign | `akmon bundle sign <bundle> --key k.pk8` | Adds an offline Ed25519 signature over the session head. |
| Attest an operator | `akmon bundle attest <bundle> --key op.pk8 --operator-id you@org --role approver` | Records the accountable person behind a session, signed separately from the head signature. |
| Verify | `akmon bundle verify <bundle> --verify-key k.pub --require-signature` | Checks integrity, the signature, any operator attestation, and the capture level. |
| Verify on its own | `agef-verify <bundle> --verify-key k.pub` | A small, separate binary for auditors. It does not need the full Akmon install. |
| Prove with openssl | `akmon bundle prove-openssl <bundle> --verify-key k.pub --out-dir proof` | Writes out the statement, signature, and public key so anyone can check the signature with plain `openssl`. |

Other commands round out the lifecycle: `bundle export` and `bundle import`, `inspect`, `diff`, `replay` (deterministic playback of sessions Akmon produced itself), `redact`, `audit`, `evidence`, `verify`, `policy`, and `doctor`.

## Quickstart

You can install Akmon a few ways.

On macOS or Linux with Homebrew:

```bash
brew tap radotsvetkov/akmon
brew install akmon         # the CLI
brew install agef-verify   # the standalone verifier for auditors
```

Or grab the prebuilt binaries directly. Each GitHub release attaches prebuilt `akmon` and `agef-verify` binaries for Linux and macOS, plus a `SHA256SUMS` file so you can check what you downloaded. For Linux on x86_64:

```bash
base=https://github.com/radotsvetkov/akmon/releases/latest/download
curl -LO $base/akmon-linux-x86_64
curl -LO $base/agef-verify-linux-x86_64
curl -LO $base/SHA256SUMS

# Check the downloads against the published checksums before installing.
sha256sum --check --ignore-missing SHA256SUMS

chmod +x akmon-linux-x86_64 agef-verify-linux-x86_64
mv akmon-linux-x86_64 ~/bin/akmon
mv agef-verify-linux-x86_64 ~/bin/agef-verify
```

On macOS the file names are `akmon-darwin-arm64` and `agef-verify-darwin-arm64` for Apple silicon, or the `-x86_64` variants for Intel, and you check them with `shasum -a 256 --check --ignore-missing SHA256SUMS`.

Or build from source on any platform:

```bash
cargo install --git https://github.com/radotsvetkov/akmon akmon
cargo install --git https://github.com/radotsvetkov/akmon agef-verify
```

Here is a full run, from any agent's OpenTelemetry trace to a proof a stranger can check.

```bash
# 1. Make a signing key. Keep the private half secret and hand out only the .pub.
akmon bundle keygen --out signer.pk8 --public-out signer.pub

# 2. Turn a real trace into a session, then a portable bundle.
akmon otel import trace.json --journal ./journal
akmon bundle export <session-id> --journal ./journal --output audit.akmon

# 3. Sign it.
akmon bundle sign audit.akmon --key signer.pk8

# 4. Verify integrity, the signature, and the capture level.
akmon bundle verify audit.akmon --verify-key signer.pub --require-signature

# 5. Prove it with openssl alone, no Akmon involved.
akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin -in proof/statement.bin -sigfile proof/signature.bin
```

Use OpenSSL 3.x for the verify step. The `openssl` that ships with macOS is LibreSSL, which cannot verify Ed25519. Install OpenSSL 3.x first, for example with `brew install openssl@3`.

There is a full walkthrough in the docs: [from an OTEL trace to an offline openssl proof](docs/src/tutorials/otel-to-openssl-walkthrough.md).

## How much a session captures

Akmon is explicit about how complete each record is, and it signs that level into the record so nobody can quietly overstate it.

Full capture (`capture_level = full`) comes from Akmon's own agent. It keeps the prompts, responses, and tool calls, so the session replays deterministically with `akmon replay`.

Structural capture (`capture_level = structural`) comes from importing another agent's OpenTelemetry trace. You get the shape of the session, its provider calls, tool calls, metadata, and whatever content the trace happened to include, but not a byte-for-byte replay. `akmon bundle verify --require-capture full` fails on these on purpose, and `akmon replay` refuses them.

An imported trace is never dressed up as a full recording. Keeping that line clear is the whole reason a trust layer is worth having.

## How it compares to Microsoft

Microsoft ships a solid governance runtime, the open-source Agent Governance Toolkit, which has been generally available since April 2026, and a strong tamper-evident cloud ledger in Azure Confidential Ledger. As of June 2026, though, no single Microsoft product gives you one portable, self-contained evidence artifact that is signed with an asymmetric key (Ed25519), checkable by a stranger who has nothing from Microsoft installed, deterministically replayable, and able to sit on top of any agent. The Toolkit's tamper-evidence is a hash chain plus HMAC, with no asymmetric signature and no standalone verifier. Confidential Ledger's signed Merkle receipts are genuinely good, but they are tied to Azure and are not aware of agents. Microsoft's own Foundry documentation says its traces cannot support full replay.

That gap is where Akmon fits. It is portable, signed, cloud-independent, agent-aware, and replayable.

Akmon is not trying to replace any of that, and there are places where Microsoft is clearly ahead. Its distribution is one: Purview and the Copilot Control System are already in every Microsoft 365 tenant. Confidential Ledger's offline-verifiable receipts are another, and so is Microsoft's weight in the standards bodies. A single tool will not match those. Akmon is meant to complement them: seal what Purview captures, and export and verify what Foundry traces.

## Compliance

Akmon is built to help you produce evidence for frameworks like the EU AI Act (Article 12 and Annex IV logging, with high-risk obligations from 2 August 2026), the NIST AI Risk Management Framework (MEASURE 2.8), and SOC 2 (CC7.x and CC8.1). It is not a compliance certification, and it does not guarantee compliance. Treat any mapping from AGEF evidence to specific controls as something to validate with your own compliance and legal teams.

## The bundled agent

Akmon ships with its own agent (`akmon`, `akmon chat`, and `akmon --task ...`). It has typed permission checks for file writes, shell, and network, runs local or hosted models, supports policy profiles, and governs MCP servers. It exists mainly as the reference producer, the way to get full-capture sessions that replay deterministically. It is not trying to out-code Cursor or Claude Code. The value Akmon offers is the evidence layer, and that works no matter which agent you prefer.

```bash
# A session from Akmon's own agent, with its verification pipeline.
akmon --yes --output json --task "apply rustfmt fixes only" | tee run.json
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon evidence verify .akmon/evidence/<session-id>.json
akmon slo verify .akmon/evidence/<session-id>.json --strict
```

## The evidence format (AGEF)

Akmon's records follow the [AGEF specification](https://github.com/radotsvetkov/agef), a portable, content-addressed, tamper-evident format for AI-agent session evidence. Akmon implements AGEF v0.1.3. That version adds two optional pieces on top of the hash chain: offline Ed25519 signatures in `manifest.signatures[]`, which make a record attributable to a key, and operator attestations in `manifest.operator_attestations[]`, which record the accountable person behind a session. Both are optional, so a plain bundle stays small and older readers keep working.

## Documentation

- Project site: [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/)
- Hosted handbook: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)
- Introduction: [docs/src/introduction.md](docs/src/introduction.md)
- Tutorial, from an OTEL trace to an openssl proof: [docs/src/tutorials/otel-to-openssl-walkthrough.md](docs/src/tutorials/otel-to-openssl-walkthrough.md)
- Reference for `bundle keygen`: [docs/src/reference/bundle-keygen.md](docs/src/reference/bundle-keygen.md)
- Reference for `bundle attest`: [docs/src/reference/bundle-attest.md](docs/src/reference/bundle-attest.md)
- Reference for `bundle prove-openssl`: [docs/src/reference/bundle-prove-openssl.md](docs/src/reference/bundle-prove-openssl.md)

## Contributing

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](SECURITY.md)

## License

Apache-2.0 only. See [LICENSE](LICENSE).

## Where the name comes from

Akmon is an old Greek word for anvil. The idea is to shape difficult work with pressure and precision while keeping control over every strike: the permissions, the tamper-evident audit trail, and the evidence anyone can verify.
