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

**Prove what your AI agent did, to someone who doesn't trust you.**

Sooner or later someone asks you to prove what an AI agent actually did: an auditor, a customer's security team, an incident reviewer, a regulator. It may be years after the session. They don't trust you, they won't install your tools, and "here are our logs" doesn't help when those logs are mutable spans you produced yourself. Akmon turns an agent session into **one sealed file**. That file is a signed, tamper-evident [AGEF](https://github.com/radotsvetkov/agef) bundle, and the person asking can verify it **offline**: with the small `agef-verify` binary, or with nothing but stock OpenSSL 3. It works on top of any agent that emits OpenTelemetry GenAI traces, and on Akmon's own bundled reference agent.

Website: [radotsvetkov.github.io/akmon](https://radotsvetkov.github.io/akmon/) · Handbook: [radotsvetkov.github.io/akmon/docs](https://radotsvetkov.github.io/akmon/docs/) · Current release: **v3.0.0** ([what changed](CHANGELOG.md))

[![CI](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml/badge.svg)](https://github.com/radotsvetkov/akmon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org)

## 60-second quickstart

Seal: you, the producer.

```bash
# 1. One-time: make an Ed25519 signing key. Keep signer.pk8 secret, hand out signer.pub.
akmon bundle keygen --out signer.pk8 --public-out signer.pub

# 2. Seal an agent session. The input is an OTLP/JSON trace from any agent, or a journal session id.
#    This one command imports, bundles, verifies integrity, signs and self-verifies.
akmon seal trace.json --key signer.pk8 --output audit.akmon
# sealed: audit.akmon
#   capture level: full
#   integrity: OK | signature: verified | self-verify: passed
# It also writes audit.akmon.pub (the signer public key, hex) and prints the exact
# agef-verify commands to hand over. Add --proof-dir proof for OpenSSL artifacts.
```

Verify: the auditor, on their own machine, with no Akmon install.

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature
# verified bundle: integrity OK; signature verified by key_id=c452d258…: audit.akmon
#   capture: FULL (OTEL import; message content captured)
#   ...

# A plain-language report to attach to a ticket or audit file:
agef-verify audit.akmon --verify-key signer.pub --require-signature --format markdown > report.md
```

Send the public key over a **separate, trusted channel** (not next to the bundle). If an attacker can swap both the bundle and the key, a signature proves nothing.

**No Akmon at all? Use OpenSSL.** Add `--proof-dir proof` to `akmon seal`, or run `akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof` on a bundle you already have. The verifier then runs:

```bash
openssl pkeyutl -verify -pubin -inkey proof/pubkey.pem -rawin \
  -in proof/statement.bin -sigfile proof/signature.bin
# Signature Verified Successfully
```

This shows that the key holder signed session head *H*. `agef-verify` is the part that ties *H* to the bundle's contents. Use OpenSSL 3.x: the `openssl` that ships with macOS is LibreSSL, which cannot verify Ed25519 (`brew install openssl@3`).

The step-by-step version, with a real OpenTelemetry fixture, is the [OTEL trace → openssl proof walkthrough](docs/src/tutorials/otel-to-openssl-walkthrough.md).

## Install

**Homebrew** (macOS, Linux):

```bash
brew tap radotsvetkov/akmon
brew install akmon          # the CLI
brew install agef-verify    # the standalone verifier, which is all an auditor needs
```

**Prebuilt binaries + checksums.** Every GitHub release ships `akmon` and `agef-verify` for Linux x86_64 and macOS (arm64, x86_64), plus `SHA256SUMS` and a CycloneDX SBOM (`akmon.cdx.json`):

```bash
base=https://github.com/radotsvetkov/akmon/releases/latest/download
curl -LO "$base/akmon-linux-x86_64" -LO "$base/agef-verify-linux-x86_64" -LO "$base/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS      # macOS: shasum -a 256 --check --ignore-missing SHA256SUMS
chmod +x akmon-linux-x86_64 agef-verify-linux-x86_64
mv akmon-linux-x86_64 ~/bin/akmon && mv agef-verify-linux-x86_64 ~/bin/agef-verify
```

On macOS the file names are `akmon-darwin-arm64` / `agef-verify-darwin-arm64` (Apple silicon) or `-darwin-x86_64` (Intel).

**From source** (Rust 1.88+). The CLI's package is `akmon-cli`, which installs the `akmon` binary:

```bash
cargo install --locked --git https://github.com/radotsvetkov/akmon akmon-cli
cargo install --locked --git https://github.com/radotsvetkov/akmon agef-verify
```

## What a verification proves, and what it doesn't

| A passing `agef-verify … --verify-key K --require-signature` proves | It does **not** prove |
| --- | --- |
| **Integrity:** every event and content object hashes into the session head *H*. Nothing was changed, added, removed or reordered since *H* was computed. | **When** the session happened. Event times come from the producer's clock, and AGEF signatures do not yet cover a trusted time (see below). |
| **Authorship of H:** the holder of private key *K* signed *H*, under the domain-separated `AGEF-SIG-v1` statement. | **Who holds K.** Tying a key to a person or organization is up to you: out-of-band key exchange, a key register, a PKI. |
| **Operator sign-off** (with `--operator-key`): a second key signed an `AGEF-OPERATOR-v1` attestation naming an accountable person and role. | That the operator's name, role or org are true. They are self-asserted text, and only the key is verified. |
| **Capture honesty:** the capture level (`full` / `partial` / `structural`) is inside the signed record, so nobody can quietly overstate it. | That the agent behaved correctly, or that any legal obligation is met. |

Without `--require-signature`, a pass means **integrity only**. Anyone can build an internally consistent unsigned bundle. Fields shown as *unsigned metadata* (`created_at`, `ended_at`, producer) can be edited by whoever holds the file.

### Trusted time: an honest gap, and an interim recipe

AGEF v0.1.3 signatures do not cover a timestamp. Native RFC 3161 timestamps embedded in the bundle (`manifest.timestamps[]`, AGEF v0.1.4) are **planned, not shipped**. Until then, you can get an independent time attestation with stock OpenSSL and any RFC 3161 Time-Stamping Authority:

```bash
akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof
openssl ts -query -data proof/statement.bin -sha256 -cert -out proof/statement.tsq
curl -sS -H "Content-Type: application/timestamp-query" \
  --data-binary @proof/statement.tsq https://freetsa.org/tsr -o proof/statement.tsr
# Keep statement.tsr next to the bundle. Anyone can later check it:
openssl ts -verify -data proof/statement.bin -in proof/statement.tsr -CAfile tsa-ca.pem -untrusted tsa.crt
```

FreeTSA works for experiments. Where legal weight matters in the EU, use a **qualified** trust service provider's TSA (eIDAS Art. 41; Implementing Regulation (EU) 2025/1929; RFC 3161 / ETSI EN 319 422).

## Capture levels

Akmon records how complete a session is and signs that level into the record:

| Level | Meaning | Typical source |
| --- | --- | --- |
| `full` | Every content slot is real: request and response for each provider call, input and output for each tool call. | Akmon's reference agent; OTEL traces with content capture enabled |
| `partial` | Some content slots are real, others are missing. | OTEL traces with partial content capture. Embeddings calls never count as `full`, because their output is not a message. |
| `structural` | No content, only the shape: provider calls, tool calls, timings, token counts, errors. | OTEL traces with content capture off (the common default) |

`agef-verify --require-capture full` rejects `partial` and `structural` bundles. `akmon replay` refuses OTEL imports at any level, because an imported trace is a record, not a replayable execution. Only reference-agent sessions replay deterministically.

## The toolchain

| Command | What it does |
| --- | --- |
| `akmon seal <trace.json \| session-id> --key k.pk8` | **The one-shot path.** Import or select, export, verify integrity, sign, optionally attest, self-verify, and optionally write openssl proof artifacts (`--proof-dir`). |
| `akmon otel import <trace.json>` | OTLP/JSON GenAI trace (semconv v1.37–v1.42 structured form, the inference-details event, or the legacy ≤ v1.36 message events) → AGEF session. Prints the session id on stdout. Use `--trace-id` to pick one trace from a multi-trace file. |
| `akmon bundle keygen --out k.pk8 --public-out k.pub` | Ed25519 signing key (PKCS#8 v2, file mode `0600`) plus its public key as hex. |
| `akmon bundle export <session-id> --output s.akmon` / `bundle import` | Journal session ↔ portable `.akmon` bundle (tar.zst). |
| `akmon bundle sign <bundle> --key k.pk8` | Adds an `AGEF-SIG-v1` Ed25519 signature over the session head. Refuses a bundle that fails integrity. |
| `akmon bundle attest <bundle> --key op.pk8 --operator-id … --role …` | Adds a separately signed `AGEF-OPERATOR-v1` operator attestation. |
| `akmon bundle verify <bundle> --verify-key k.pub` | Full verification: integrity, signatures, operator attestations, capture level. `--format human\|json\|markdown`. |
| `agef-verify <bundle> --verify-key k.pub` | The same verification as a small standalone binary for auditors and CI. |
| `akmon bundle prove-openssl <bundle> --verify-key k.pub --out-dir proof` | Verifies the bundle first, then writes `statement.bin`, `signature.bin` and `pubkey.pem` for a check with stock `openssl`. |
| `akmon inspect <session-id>` | Read a session's events and resolve content objects. |
| `akmon diff <a> <b>` | Structural and field-level comparison of two sessions, including provider requests and retry attempts. |
| `akmon replay <session-id>` | Deterministic playback of a full-capture reference-agent session. |
| `akmon redact <session-id> --object <hash> --reason "…" --output r.akmon` | Derivative bundle with chosen objects replaced by redaction sentinels (for example to honour GDPR erasure without breaking verification). |

The trust commands share one exit-code convention: `0` passed, `1` verification failed or was refused (including corrupt, truncated or oversized archives), `2` usage or bad input, `3` I/O or environment. They also take `--format json`. `akmon seal`, the `bundle` subcommands and `agef-verify` report errors as one `ToolErrorV1` object `{tool, akmon_version, passed:false, error, category}`.

## EU AI Act: where this fits (as of 1 Nov 2026)

Akmon **helps you produce evidence**. It does not make a system compliant, it is not a certification, and nothing here is legal advice. The dates below follow Regulation (EU) 2024/1689 (the AI Act) **as amended by the Digital Omnibus on AI, Regulation (EU) 2026/1744** (OJ 24 July 2026, in force 27 July 2026).

- **High-risk obligations are deferred, not cancelled.** Chapter III duties, including **Art. 12** (automatic logging), **Art. 19** (providers keep logs for at least 6 months) and **Art. 26(6)** (deployers keep logs for at least 6 months), apply from **2 December 2027** for Annex III systems and **2 August 2028** for Annex I (product-embedded) systems.
- **Already applicable:** prohibitions and AI literacy (Feb 2025), general-purpose AI model obligations (Aug 2025, with Commission enforcement powers from 2 Aug 2026), Art. 50 transparency (2 Aug 2026; machine-readable marking has a grace period to 2 Dec 2026 for systems already on the market), and the penalties regime.
- **Where a sealed bundle helps:** tamper-evident session logs for Art. 12-style record-keeping, retained under your own Art. 19/26(6) retention (pair bundles with WORM or object-lock storage); **Annex IV point 2(g)**, *"test logs and all test reports dated and signed by the responsible persons"*, where a signed test-session bundle plus an operator attestation is a good fit (for the "dated" part, add a trusted timestamp as described above); and **Art. 73** serious-incident reporting, where a frozen, signed bundle helps show records were not altered before the authorities were informed (Art. 73(6)). Commission guidance on Art. 73 is still a draft.
- **What it does not do:** operator attestation helps you **evidence** human oversight and sign-off. It does not by itself satisfy Art. 14. The Art. 12(3) minimum log fields apply only to remote biometric identification systems (Annex III point 1(a)). AI agents are not a separate category under the Act (AI Office FAQ, preliminary).
- **Standards are not there yet:** ISO/IEC 24970 (AI system logging) is at FDIS, prEN 18229-1 (AI trustworthiness, logging) is at CEN enquiry, and EN 18286 (QMS) was published in 2026. None gives a presumption of conformity for Art. 12 today.
- **Adjacent rules that already apply:** DORA RTS (EU) 2024/1774 Art. 12 (tamper-protected, time-synchronised, retained ICT logs), NIS2 Implementing Regulation (EU) 2024/2690 §3.2, and Cyber Resilience Act vulnerability and incident reporting (since 11 Sep 2026).

The handbook's [compliance page](docs/src/concepts/compliance.md) has the details, sources and caveats. Check the mapping with your own legal and compliance teams.

## How it compares (as of Nov 2026)

Akmon does not replace your agent or your governance platform. It adds a portable, asymmetrically signed record that a stranger can verify offline.

- **Microsoft Agent Governance Toolkit**: a governance runtime whose documentation now describes a Merkle-chained, tamper-evident audit log. That is real tamper-evidence, but it is not an asymmetric signature a stranger can check with only a public key, and there is no standalone third-party verifier or openssl-checkable proof.
- **Azure Confidential Ledger**: genuinely strong signed receipts, but tied to the Azure service and not agent-aware.
- **Vendor tracing (for example Microsoft Foundry)**: built for live observability. Microsoft's own docs say Foundry traces cannot support full replay.
- **Plain OpenTelemetry**: the right way to *emit* agent telemetry, and Akmon imports it. But spans are unsigned and mutable once exported.

Where those tools lead: distribution, ecosystem integration and standards presence. The realistic stack is layered: produce and govern in your platform, then seal the session into a record that can leave that trust domain. There is a longer, sourced comparison in the [handbook](docs/src/comparison.md).

## The bundled reference agent

Akmon still ships its own terminal agent (`akmon`, `akmon chat`, `akmon --task "…"`), with typed permission gates, a path-normalizing policy engine, a sandbox, and MCP governance. In v3 it has one role: it is the **full-capture reference producer**, the source of sessions that record every prompt, response and tool call and so can be replayed and diffed deterministically. It is not trying to out-code other agents.

```bash
akmon --yes --output json --task "apply rustfmt fixes only" > run.json   # records a journal session
akmon seal <session-id> --key signer.pk8 --output run.akmon             # seal it like any other
```

## The evidence format (AGEF)

Akmon implements the [AGEF specification](https://github.com/radotsvetkov/agef) **v0.1.3**: a portable, content-addressed, hash-linked format for AI-agent session evidence. A bundle is a `tar.zst` archive with `manifest.json`, `events.bin` (CBOR-framed events) and `objects/<sha256>`. Signatures (`manifest.signatures[]`, `AGEF-SIG-v1`) and operator attestations (`manifest.operator_attestations[]`, `AGEF-OPERATOR-v1`) are optional and never enter the hash chain, so integrity verification works without any key.

## Documentation

- [Introduction](docs/src/introduction.md) and [Quickstart](docs/src/getting-started/quickstart.md)
- [Verifying evidence](docs/src/concepts/verifying-evidence.md) and the [trust model](docs/src/concepts/trust-model.md)
- [`akmon seal` reference](docs/src/reference/seal.md), [`otel import`](docs/src/reference/otel-import.md), [`bundle sign`](docs/src/reference/bundle-sign.md), [capture levels](docs/src/concepts/capture-levels.md)
- [`agef-verify` reference](docs/src/reference/agef-verify.md), [`bundle verify`](docs/src/reference/bundle-verify.md), [`bundle keygen`](docs/src/reference/bundle-keygen.md), [`bundle attest`](docs/src/reference/bundle-attest.md), [`bundle prove-openssl`](docs/src/reference/bundle-prove-openssl.md)
- [Compliance (EU AI Act and adjacent rules)](docs/src/concepts/compliance.md)
- Use cases: [operator sign-off](docs/src/use-cases/operator-sign-off.md), [air-gapped audit](docs/src/use-cases/air-gapped-audit.md), [release evidence pack](docs/src/use-cases/release-evidence-pack.md)

## What changed in 3.0

v3.0.0 is the **focus** release. It adds `akmon seal`, an auditor Markdown report (`--format markdown`) and hardened verification (stricter archive parsing, terminal-escape sanitizing, explicit integrity-only vs. signature-verified verdicts). It also archives everything that wasn't evidence: `akmon import/export/spec/scout/slo/sign`, the semantic index, and subagents. Some JSON and CLI changes are breaking. See the [CHANGELOG](CHANGELOG.md) and the [v3.0.0 release notes](docs/src/releases/v3.0.0.md). Archived code is under [`archive/`](archive/README.md) and the tag `archive/pre-focus-v2.2.1`.

## Contributing, security, license

- [CONTRIBUTING.md](CONTRIBUTING.md) · [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) · [SECURITY.md](SECURITY.md) (please report vulnerabilities privately)
- Apache-2.0 only. See [LICENSE](LICENSE).

*Akmon* (ἄκμων) is Greek for anvil: the surface that work is shaped against, under pressure, one strike at a time.
