# Capabilities reference

Documented for Akmon `2.2.0`.

This page is a practical map of what Akmon can do. Akmon is a producer-agnostic, tamper-evident evidence and verification layer for AI agents. The verification chain is the core capability; the bundled reference agent is one way to feed it, not the headline.

## Evidence and verification (the core)

| Capability | Command | Why it matters |
| --- | --- | --- |
| Import any agent's trace | `akmon otel import <trace.json>` | Brings OpenTelemetry GenAI traces (v1.37 structured and legacy v1.36-and-earlier message-event form) into AGEF; records an honest capture level |
| Content-addressed, hash-linked record | (format) | SHA-256 objects and a hash-linked event chain make the session tamper-evident by construction |
| Generate a signing key | `akmon bundle keygen` | Ed25519 PKCS#8 v2 key that `akmon bundle sign` accepts; `openssl genpkey` cannot produce one |
| Offline signature over the head | `akmon bundle sign` | Adds an Ed25519 signature over the session head, the `AGEF-SIG-v1` statement |
| Operator attestation | `akmon bundle attest` | Records a separately signed operator-identity claim; trust attaches to the key, not the name |
| Verify integrity, signature, capture | `akmon bundle verify` | Checks the chain, the signature, operator attestation, and `--require-capture full` |
| Standalone verifier | `agef-verify` | Verifies a bundle with no full Akmon install, for auditors |
| Offline proof with plain openssl | `akmon bundle prove-openssl` | Emits `statement.bin`, `signature.bin`, `pubkey.pem`, and the exact `openssl` command; needs OpenSSL 3.x |
| Portable transport | `akmon bundle export` / `bundle import` | Moves a sealed session between machines and tools |
| Inspect, compare, redact | `akmon inspect`, `akmon diff`, `akmon redact` | Read a bundle, compare two sessions structurally and by field, remove sensitive bytes while preserving structure |

The standard pattern for an imported session: import a trace, export and sign the bundle, verify integrity and signature, then emit an `openssl` proof a stranger can check.

## Capture honesty

Akmon never overstates how much it captured.

| Source | Capture level | Replay | `--require-capture full` |
| --- | --- | --- | --- |
| Akmon's own reference agent | `full` | Replays deterministically | Passes |
| OpenTelemetry import | `structural` | Refused | Fails (by design) |

## The bundled reference agent (gold-fidelity producer)

Akmon ships a coding agent that produces full-capture sessions. It is the reference producer, not the product. Its own-agent verification surface:

- `akmon audit verify`, `akmon evidence verify`, and `akmon verify` for the on-disk journal, audit chain, and evidence artifact,
- `akmon replay` for deterministic playback,
- `akmon slo verify --strict` for reliability thresholds.

### Operating modes (reference agent)

| Mode | Command | Best use case |
| --- | --- | --- |
| Interactive | `akmon chat` | supervised iterative implementation |
| Headless | `akmon --yes --task "..."` | CI and automation |
| JSON reporting | `--output json` | machine-readable orchestration |
| Plan-only | `--plan` | read-only scoping before edits |
| Architect | `--architect` | plan and implement with a model split |
| Spec workflow | `akmon spec ...` | structured requirements, design, and tasks |

## Runtime and packaging

| Capability | Why it matters |
| --- | --- |
| Single Rust binary | predictable behavior across laptop, SSH host, CI runner |
| Standalone verifier binary (`agef-verify`) | auditors verify without installing the full agent |
| Optional feature set | choose slim or full builds by environment needs |
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
- no certification or compliance guarantee. Akmon helps you produce evidence for frameworks like the EU AI Act, NIST AI RMF, and SOC 2; validate fit with your own legal and compliance teams.

Next steps: [tutorials overview](../tutorials/overview.md), [reviewer flow](../concepts/reviewer-flow.md), [security model](../features/security.md).
