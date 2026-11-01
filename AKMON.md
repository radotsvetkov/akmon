# Akmon

Akmon seals AI agent sessions into signed,
tamper-evident AGEF bundles that anyone can
verify offline (`akmon seal`, `agef-verify`,
`openssl`). The bundled agent is the
full-capture reference producer.

## Layout (crates/)

Trust path:
- akmon-journal: object store, merkle session graph
- akmon-bundle: AGEF archive, verify, report, render,
  Ed25519 signing (AGEF-SIG-v1, AGEF-OPERATOR-v1)
- agef-verify: standalone verifier binary
- akmon-otel: OTel GenAI trace import
- akmon-replay, akmon-diff: replay and compare
- akmon-cli: `akmon` (seal, bundle, otel, verify,
  inspect, redact, diff, replay)

Reference agent:
- akmon-core: policy, sandbox, FSM, secrets
- akmon-models: LLM providers
- akmon-tools: Tool trait, built-in tools
- akmon-query: AgentSession loop, journaling
- akmon-tui, akmon-config: UI, ~/.akmon/config.toml

archive/ holds retired code; it is not built.

## Conventions

- Never change AGEF substrate bytes or EventKind
- No unwrap() outside tests
- Every public item has a doc comment
- File paths go through sandbox validation
- Sanitize untrusted text before printing it
- Secrets never appear in logs or argv
- Gates: fmt, clippy --all-targets -D warnings,
  cargo test --workspace --no-default-features
