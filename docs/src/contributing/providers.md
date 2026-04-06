# Adding a provider

Providers live in **`akmon-models`**. Each backend implements the **`LlmProvider`** trait (streaming completions, auth, and provider-specific request shaping).

## Steps (overview)

1. **Backend module** — add a submodule under `crates/akmon-models/src/` for the API (HTTP, signing, streaming parse).
2. **Implement `LlmProvider`** — map Akmon’s generic message/tool format to the vendor API; handle token usage and errors.
3. **Wire config** — extend `akmon-config` / CLI parsing for keys, base URLs, and model id conventions.
4. **Detection** — update provider auto-detection order (env vars, flags) in CLI/config.
5. **Tests** — unit-test request JSON and response parsing with fixtures; avoid live API calls in CI.

## Conventions

- No `.unwrap()` in library code; use typed errors (`thiserror`).
- Never log secrets; use existing `Secret` types from `akmon-core` where applicable.
- Document new flags and env vars in user docs (`docs/src/providers/`, [CLI](./../reference/cli.md)).

## See also

- [Architecture](./architecture.md) — crate graph and `LlmProvider`.
- [Development setup](./setup.md) — build and test commands.
