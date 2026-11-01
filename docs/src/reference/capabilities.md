# Capabilities reference

Documented for Akmon `3.0.0`.

A map of what Akmon 3.0.0 can do. The core job is sealing an agent session into one signed file a stranger can verify offline; the bundled reference agent is one way to produce sessions.

## Evidence and verification (the core)

| Capability | Command | Why it matters |
| --- | --- | --- |
| Seal in one step | `akmon seal <trace.json\|session-id> --key <pk8>` | Import or select, export, sign, optionally attest, self-verify, and print the hand-over commands |
| Import any agent's trace | `akmon otel import <trace.json>` | Brings OpenTelemetry GenAI traces (structured form v1.37 to v1.42, and the legacy v1.36-and-earlier message-event form) into AGEF; records an honest capture level |
| Content-addressed, hash-linked record | (format) | SHA-256 objects and a hash-linked event chain make the session tamper-evident by construction |
| Generate a signing key | `akmon bundle keygen` | Ed25519 PKCS#8 v2 key that `akmon bundle sign` accepts; `openssl genpkey` cannot produce one |
| Offline signature over the head | `akmon bundle sign` | Adds an Ed25519 signature over the session head, the `AGEF-SIG-v1` statement |
| Operator attestation | `akmon bundle attest` | Records a separately signed operator-identity claim; trust attaches to the key, not the name |
| Verify integrity, signature, capture | `akmon bundle verify` | Checks the chain, the signature, operator attestation, and `--require-capture full`; `--format markdown` writes an auditor report |
| Standalone verifier | `agef-verify` | Verifies a bundle with no full Akmon install, for auditors |
| Offline proof with plain openssl | `akmon bundle prove-openssl` | Emits `statement.bin`, `signature.bin`, `pubkey.pem`, and the exact `openssl` command; needs OpenSSL 3.x |
| Portable transport | `akmon bundle export` / `bundle import` | Moves a sealed session between machines and tools |
| Inspect, compare, redact | `akmon inspect`, `akmon diff`, `akmon redact` | Read a bundle, compare two sessions structurally and by field, remove sensitive bytes while preserving structure |

The standard pattern: `akmon seal`, hand over the bundle and (separately) the public key, and let the verifier run `agef-verify` or OpenSSL.

## Capture honesty

Akmon never overstates how much it captured.

| Source | Capture level | Replay | `--require-capture full` |
| --- | --- | --- | --- |
| Akmon's own reference agent | `full` | Replays deterministically | Passes |
| OpenTelemetry import with all message content | `full` | Refused (imports are evidence records, not executions) | Passes |
| OpenTelemetry import with some content | `partial` | Refused | Fails |
| OpenTelemetry import, metadata only | `structural` | Refused | Fails |

See [Capture levels](../concepts/capture-levels.md).

## The bundled reference agent (full-capture producer)

Akmon ships a coding agent that produces full-capture sessions. It is the reference producer, not the product. Its own-agent verification surface:

- `akmon verify` for a session in the on-disk journal,
- `akmon replay` for deterministic playback,
- `akmon diff` to compare two sessions.

### Operating modes (reference agent)

| Mode | Command | Best use case |
| --- | --- | --- |
| Interactive | `akmon chat` | supervised iterative implementation |
| Headless | `akmon --yes --task "..."` | CI and automation |
| JSON reporting | `--output json` | machine-readable orchestration |
| Plan-only | `--plan` | read-only scoping before edits |
| Architect | `--architect` | plan and implement with a model split |

## Runtime and packaging

| Capability | Why it matters |
| --- | --- |
| Single Rust binary | predictable behavior across laptop, SSH host, CI runner |
| Standalone verifier binary (`agef-verify`) | auditors verify without installing the full agent |
| Terminal-first UX | works where editor plugins are unavailable |

## Model and provider support (reference agent)

The bundled agent supports local and cloud providers:

- Ollama (offline and local),
- Anthropic,
- OpenAI-compatible providers,
- OpenRouter, Groq, Azure, Bedrock.

Model selection is per-task, which keeps cost and capability tuning an operator decision rather than a tooling lock-in.

## Policy and safety capabilities

- permission-gated side effects,
- write diff confirmation flows,
- sandboxed filesystem boundaries,
- auditable tool and policy events.

## Cost and observability capabilities

- token and cache visibility in the UI,
- cost estimates and run summaries,
- JSONL audit trail for runtime evidence.

## Automation capabilities

- headless runs with budget caps,
- structured JSON run output,
- a script-friendly command model for batch operations and CI gates.

## Known non-goals

- no hosted SaaS runtime (you run it),
- no mandatory IDE dependency,
- no guarantee that third-party model APIs are available,
- no trusted timestamp inside the signature yet (see [Trusted timestamps](../guides/trusted-timestamps.md)),
- no certification or compliance guarantee. Akmon helps you produce evidence for frameworks like the EU AI Act, NIST AI RMF, and SOC 2; see the [Compliance crosswalk](../concepts/compliance.md) and validate fit with your own legal and compliance teams.

Next steps: [quick start](../getting-started/quickstart.md), [reviewer flow](../concepts/reviewer-flow.md), [security model](../features/security.md).
