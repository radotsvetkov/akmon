# Tutorials overview

Documented for Akmon `2.2.0`.

Akmon is a producer-agnostic, tamper-evident evidence and verification layer for AI agents. It sits on top of whatever agent you run, an OpenTelemetry-instrumented agent of your own or Akmon's bundled reference agent, and turns each session into a portable, content-addressed, cryptographically signed record. A third party can verify that record offline with nothing but `openssl`: no Akmon install, no cloud, no need to trust whoever produced it.

These tutorials are organized by perspective of usage. Find the role you are working in and start there. The trust chain is the same across roles; what differs is what you produce, what you sign, and what you check.

## Before you start

Complete:

- [Installation](../getting-started/installation.md)
- [Quick start](../getting-started/quickstart.md), which walks the full trust flow: keygen, otel import, sign, verify, prove-openssl, openssl
- optional provider setup if you plan to run the bundled reference agent

Recommended baseline command:

```bash
akmon --version
```

## The developer producing evidence

You run an agent and want a verifiable record of what it did. If your agent is OpenTelemetry-instrumented, you import its trace; if you use Akmon's reference agent, you get a full-capture, replayable recording. Either way you end with a signed bundle a reviewer can check.

| Tutorial | Outcome |
| --- | --- |
| [Third-party OTEL trace to offline openssl proof](./otel-to-openssl-walkthrough.md) | Import an OpenTelemetry trace, sign it, verify it, and prove the signature with plain `openssl`, no Akmon install on the verifier's side. This is the producer-agnostic headline path and needs no Akmon agent at all. |
| [Local-first developer flow (Ollama)](./local-first-ollama.md) | Run the reference agent fully local and air-gap-friendly, and still emit a portable, signed, independently verifiable record. |

## The operator or approver signing off

You are accountable for a change and must put your name, backed by a key, behind a session. You attest the bundle with an operator key, and a reviewer can later confirm offline which key claimed it. Trust attaches to the key, not to the self-asserted identity string.

| Page | Outcome |
| --- | --- |
| [Record who approved an AI change](../use-cases/operator-sign-off.md) | Generate an operator key, attest a bundle with your operator id and role, distribute the public key out of band, and let a verifier require that specific key. |

## The auditor verifying

You received a `.akmon` bundle and a public key, possibly on an air-gapped machine, and you do not run Akmon. You check integrity, signature, operator attestation, and capture level, and you read the outcome honestly.

| Page | Outcome |
| --- | --- |
| [Verify evidence on an air-gapped machine](../use-cases/air-gapped-audit.md) | Verify with `akmon bundle verify`, with the standalone `agef-verify`, or with plain `openssl` against `prove-openssl` artifacts, and read outcomes correctly. |

## The release or compliance owner assembling a pack

You gather the sessions for a regulated release, sign them, capture operator sign-off, and hand a reviewer a pack they can independently check. You require the signature and capture level the obligation actually needs.

| Page | Outcome |
| --- | --- |
| [Assemble a signed evidence pack for a regulated release](../use-cases/release-evidence-pack.md) | Produce or import sessions, export and sign bundles, attest operator sign-off, and verify with `--require-signature` and `--require-capture`. |
| [CI headless governance flow](./ci-headless-governance.md) | Make CI fail unless a signed, verified evidence bundle exists, on top of the audit, evidence, and SLO gates. |
| [Enterprise policy rollout](./enterprise-policy-rollout.md) | Stage `dev`, `staging`, then `prod` policy profiles, tie the recorded `policy_hash` to evidence, and hand reviewers a signed bundle. |

## Honest scope

Akmon helps you produce evidence. A session run under the bundled reference agent is `full` capture and replays; an OpenTelemetry import is `structural`, the shape of the session and not a full recording, so `akmon bundle verify --require-capture full` fails on it and replay refuses it. Akmon can help you produce evidence for obligations such as EU AI Act Article 12 and Annex IV record-keeping, NIST AI RMF MEASURE 2.8, and SOC 2 CC7.x and CC8.1, but it is not a certification and does not guarantee compliance. Validate any regulatory use with your own legal and compliance teams. See [Compliance and evidence](../concepts/compliance.md) for the boundary.

## Troubleshooting prerequisites

- If `openssl` cannot verify a proof on macOS, you are on LibreSSL. Use OpenSSL 3.x.
- If `akmon bundle sign` rejects a key, regenerate it with `akmon bundle keygen` (it produces the required PKCS#8 v2 form).
- If `--require-capture full` fails on an imported session, that is expected. Imports are `structural`.
- If provider calls fail in the reference agent, verify keys and model names first.

Related: [Glossary](../concepts/glossary.md), [Regulated reviewer flow](../concepts/reviewer-flow.md), [Verifying evidence](../concepts/verifying-evidence.md), [Trust and threat model](../concepts/trust-model.md), [headless mode](../usage/headless.md).
