# Akmon

Akmon seals AI agent sessions into signed,
tamper-evident AGEF bundles that anyone can
verify offline. The bundled coding agent is
the full-capture reference producer.

## Project structure

Trust path:
- akmon-journal — object store, merkle session graph
- akmon-bundle — AGEF bundles, verify, Ed25519 signing
- agef-verify — standalone verifier binary
- akmon-otel — OTel GenAI trace import
- akmon-replay, akmon-diff — replay and compare sessions
- akmon-cli — `akmon` binary (bundle, otel, verify, inspect, redact)

Reference agent:
- akmon-core — policy, sandbox, FSM, secrets
- akmon-models — LLM providers
- akmon-tools — Tool trait, built-in tools
- akmon-query — AgentSession loop, journaling
- akmon-tui, akmon-config — UI, ~/.akmon/config.toml

Archived code lives in archive/ (not built).

## Conventions

- Never change AGEF substrate bytes
- No unwrap() outside tests
- Every public item has a doc comment
- File paths go through sandbox validation
- Secrets never appear in logs
