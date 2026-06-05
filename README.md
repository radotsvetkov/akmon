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

Akmon is a **producer-agnostic, tamper-evident evidence and verification layer for AI agents.** Point it at whatever agent you already run — through OpenTelemetry, or with Akmon's own reference agent — and every session becomes a portable, content-addressed, cryptographically **signed**, independently **verifiable** artifact. The sharpest part: a third party can verify a signature **offline with nothing but `openssl`** — no Akmon install, no cloud, no trust in whoever produced it.

**Website:** [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/) · **Docs:** [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions)
[![Passed tests](https://img.shields.io/github/actions/workflow/status/radotsvetkov/akmon/ci.yml?branch=main&label=passed%20tests)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## Why this exists

When an AI agent makes a change, you may later have to **prove what it did** — to a regulator, a security reviewer, or an incident team — possibly years later, to someone who does not trust you and does not run your tools. Most agent telemetry can't carry that weight: it lives in process memory or in mutable, unsigned spans, and "the AI did it" is not an answer. The EU AI Act's high-risk logging obligations (Art. 12 + Annex IV) begin applying **August 2, 2026**.

Akmon's bet is that the **evidence is the deliverable.** It takes a session — yours or any OpenTelemetry-instrumented agent's — and turns it into a sealed record whose integrity and authorship a stranger can check independently, with standard tools, on an air-gapped laptop.

## The trust toolchain

Every command below is shipped and tested. The center of gravity is the verification chain, not the agent.

| Step | Command | What it does |
| --- | --- | --- |
| **Import any agent** | `akmon otel import <trace.json>` | Turn an OpenTelemetry GenAI trace (v1.37 structured **or** legacy ≤v1.36 message-event form) into an AGEF session. Honest `capture_level` (`full` / `structural`). |
| **Generate a key** | `akmon bundle keygen --out k.pk8 --public-out k.pub` | Create an Ed25519 signing key (PKCS#8 v2). |
| **Sign** | `akmon bundle sign <bundle> --key k.pk8` | Offline Ed25519 signature over the session head (`AGEF-SIG-v1`). |
| **Verify** | `akmon bundle verify <bundle> --verify-key k.pub --require-signature` | Integrity + signature + honest capture-level enforcement (`--require-capture full`). |
| **Verify standalone** | `agef-verify <bundle> --verify-key k.pub` | A tiny separate binary for auditors — no full Akmon needed. |
| **Prove with only openssl** | `akmon bundle prove-openssl <bundle> --verify-key k.pub --out-dir proof` | Emit `statement.bin` / `signature.bin` / `pubkey.pem` so anyone verifies the signature with stock `openssl` — **no Akmon, no cloud.** |

Supporting commands carried across the lifecycle: `bundle export`/`import`, `inspect`, `diff`, `replay` (deterministic playback of **own-agent** sessions), `redact`, `audit`, `evidence`, `verify`, `policy`, `doctor`.

## Quickstart

Install one of two ways:

```bash
# Prebuilt `akmon` binary — Linux x86_64 (macOS: akmon-darwin-arm64 / akmon-darwin-x86_64)
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 -o ~/bin/akmon
chmod +x ~/bin/akmon

# Or build from source (any platform). This is also the way to get the standalone
# verifier today — the prebuilt release ships `akmon` only, not yet `agef-verify`.
cargo install --git https://github.com/radotsvetkov/akmon akmon
cargo install --git https://github.com/radotsvetkov/akmon agef-verify
```

Then take **any** OpenTelemetry-instrumented agent's trace all the way to an offline, third-party-verifiable proof:

```bash
# 1) Make a signing key (keep the private half secret; distribute only the .pub)
akmon bundle keygen --out signer.pk8 --public-out signer.pub

# 2) Turn a real agent's OTEL trace into a session, then a portable bundle
akmon otel import trace.json --journal ./journal
akmon bundle export <session-id> --journal ./journal --output audit.akmon

# 3) Sign it (offline Ed25519)
akmon bundle sign audit.akmon --key signer.pk8

# 4) Verify integrity + signature + honest capture level
akmon bundle verify audit.akmon --verify-key signer.pub --require-signature

# 5) Prove it to a stranger with ONLY openssl — no Akmon in the loop
akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin -in proof/statement.bin -sigfile proof/signature.bin
```

> Use **OpenSSL 3.x** for the verify step. macOS ships LibreSSL by default, which cannot verify Ed25519 — install OpenSSL 3.x (e.g. `brew install openssl@3`).

A full worked example is in the tutorial: [OTEL trace → offline openssl proof](docs/src/tutorials/otel-to-openssl-walkthrough.md).

## Fidelity: gold vs. structural (read this)

Akmon is honest about how much of a session it actually captured, and **signs that level into the record** so it can't be quietly overstated:

- **Gold (`capture_level = full`)** — produced by Akmon's own reference agent. The full prompt/response/tool-call content is captured, so the session **replays deterministically** (`akmon replay`).
- **Structural (`capture_level = structural`)** — produced by importing another agent's OpenTelemetry trace. You get the shape of the session (provider calls, tool calls, metadata, and whatever content the telemetry carried) but **not** byte-level/full replay. `akmon bundle verify --require-capture full` deliberately **fails** on these, and `akmon replay` refuses them.

Importing telemetry never silently masquerades as a full recording. That distinction is the whole point of a trust layer.

## How Akmon compares to Microsoft

Microsoft ships a strong governance **runtime** (the open-source Agent Governance Toolkit, GA April 2026) and a strong tamper-evident **cloud ledger** (Azure Confidential Ledger). As of June 2026, no single Microsoft product gives you a portable, self-contained, **asymmetrically signed (Ed25519)**, **offline-verifiable-by-a-stranger-with-no-Microsoft-install**, **deterministically replayable** evidence artifact that sits on top of *any* agent. The Toolkit's tamper-evidence is hash-chain + HMAC with **no asymmetric signature and no standalone verifier**; Confidential Ledger's signed Merkle receipts are excellent but **Azure-cloud-locked and not agent-aware**; Foundry's own docs state its traces **cannot support full replay**.

That seam is Akmon's wedge: **packaging + portability + cloud-independence + agent-awareness + replay.**

**Where Microsoft is stronger — and where Akmon is complementary, not a replacement:** Microsoft's distribution (Purview / Copilot Control System in every M365 tenant), Azure Confidential Ledger's genuine offline-verifiable receipts, and its ecosystem/standards weight are things a single tool won't match. Akmon is positioned to **seal what Purview captures** and **export-and-verify what Foundry traces** — not to be your governance plane. Full sourced analysis: [`docs/planning/competitive-microsoft-agt.md`](docs/planning/competitive-microsoft-agt.md).

## Compliance

Akmon is **designed to help you produce evidence** for regimes like the EU AI Act (Art. 12 / Annex IV logging; high-risk obligations from Aug 2, 2026), NIST AI RMF (MEASURE 2.8), and SOC 2 (CC7.x / CC8.1). It is **not** a compliance certification and does not guarantee compliance. The mapping from AGEF evidence to specific controls is a **draft pending legal review**: [`docs/planning/compliance-crosswalk.md`](docs/planning/compliance-crosswalk.md).

## The bundled agent

Akmon includes its own agent (`akmon`, `akmon chat`, `akmon --task ...`) with typed permission checks for writes/shell/network, local or hosted models, policy profiles, and MCP governance. It is deliberately positioned as the **reference / gold-fidelity producer** — the way to get full-replay sessions — **not** as a competitor to Cursor or Claude Code on raw coding ability. The value Akmon claims is the evidence layer that works regardless of which agent you prefer.

```bash
# Own-agent gold session + its verification pipeline
akmon --yes --output json --task "apply rustfmt fixes only" | tee run.json
akmon audit verify .akmon/audit/<session-id>.jsonl
akmon evidence verify .akmon/evidence/<session-id>.json
akmon slo verify .akmon/evidence/<session-id>.json --strict
```

## Session evidence format (AGEF)

Akmon's records conform to the [AGEF specification](https://github.com/radotsvetkov/agef) — a portable, content-addressed, tamper-evident format for AI agent session evidence. Akmon implements **AGEF v0.1.2**, including optional offline Ed25519 signatures (`manifest.signatures[]`) that turn a tamper-evident record into a third-party-attributable one.

## Documentation

- Project site: [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/)
- Hosted handbook: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/)
- Introduction: [docs/src/introduction.md](docs/src/introduction.md)
- Tutorial — OTEL → openssl proof: [docs/src/tutorials/otel-to-openssl-walkthrough.md](docs/src/tutorials/otel-to-openssl-walkthrough.md)
- Reference — `bundle keygen`: [docs/src/reference/bundle-keygen.md](docs/src/reference/bundle-keygen.md)
- Reference — `bundle prove-openssl`: [docs/src/reference/bundle-prove-openssl.md](docs/src/reference/bundle-prove-openssl.md)
- Distribution plan: [docs/planning/distribution-plan.md](docs/planning/distribution-plan.md)

## Contributing

- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of Conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](SECURITY.md)

## License

Apache-2.0 only. See [LICENSE](LICENSE).

---

### What "Akmon" means

Akmon is named after the forge/anvil idea: shape complex work with pressure and precision, while keeping control over every strike — permissions, a tamper-evident audit trail, and independently verifiable evidence.
