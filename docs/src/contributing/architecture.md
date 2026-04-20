# Architecture guide for contributors

This document explains how Akmon is organized internally and how the core agent loop works.

## Crate structure

```text
akmon/
├── crates/
│   ├── akmon-cli/      # binary entry point, args, command routing
│   ├── akmon-core/     # policies, sandbox, shared types, security primitives
│   ├── akmon-config/   # config loading and provider resolution inputs
│   ├── akmon-models/   # provider adapters and stream normalization
│   ├── akmon-tools/    # tool implementations
│   ├── akmon-query/    # agent loop, context assembly, session lifecycle
│   ├── akmon-tui/      # ratatui UI and runtime bridge
│   └── akmon-index/    # optional semantic index
```

## The agent loop (`akmon-query/src/session.rs`)

At a high level:

1. build prompt/context bundle,
2. call provider stream,
3. process deltas and stop reason,
4. execute tool calls when requested,
5. append tool results to context,
6. continue loop until model ends with no pending tools.

Stop-reason behavior:

- `ToolUse`: execute tools, continue loop,
- `EndTurn` + tool calls: execute then continue,
- `EndTurn` with no tool calls: complete run,
- `MaxTokens`: perform continuation strategy where applicable.

This loop is why Akmon behaves like an autonomous worker, not a one-response chatbot.

## Context assembly order

Effective ordering in practice:

1. project/system steering (`AKMON.md` and base system instructions),
2. optional specs/handoff context,
3. language/profile hints,
4. conversation history,
5. dynamic extras (todos/memory blocks).

The order prioritizes stable steering first, then volatile task state later.

## Provider abstraction

`akmon-models` normalizes provider-specific behavior into common stream events and model errors so `akmon-query` can remain provider-agnostic.

Responsibilities include:

- mapping provider payloads to `StreamEvent`,
- retry handling where provider-specific (for example rate limits),
- first-token/stream timeout behavior,
- provider display and model-specific heuristics.

## TUI state decomposition (`akmon-tui`)

`TuiApp` remains the composition root, but state is now split into focused internal modules under `crates/akmon-tui/src/state/`:

- `composer`: input buffer + cursor behavior (insert/paste/backspace/delete/left/right/submit),
- `overlay_state`: overlay/modal state, confirmation gate state, ask-followup state, slash autocomplete selection/suppression,
- `session_telemetry`: token/tool counters, touched files, and context-warning bookkeeping,
- `provider_runtime`: provider/runtime status such as provider label, run flags, iteration progress, stream cursor, and ollama probe.

Why this helps:

- lower regression risk by reducing mixed responsibilities in `app.rs`,
- easier targeted tests for state transitions,
- clearer ownership when adding new TUI logic without changing UX behavior.

This refactor is intentionally internal: behavior and command semantics stay the same while maintainability/testability improve.

Contributor guideline for TUI changes:

- put new input-edit semantics in `state/composer.rs`,
- put overlay transition rules in `state/overlay_state.rs`,
- put counters/usage accumulation in `state/session_telemetry.rs`,
- put runtime/provider status updates in `state/provider_runtime.rs`,
- keep `TuiApp` focused on orchestration and event routing.

## Permission system path

Before tool execution:

1. derive concrete permission requirement from tool + args,
2. evaluate policy mode (deny/auto/interative),
3. request user confirmation if needed,
4. execute tool only after allow.

This is enforced centrally in session execution flow, not left to individual tools.

## Adding a tool

1. implement `Tool` trait in `akmon-tools`,
2. define permission requirements and argument schema,
3. register in tool registry,
4. add unit tests and integration path checks,
5. document in `docs/src/reference/tools.md`.

## Common mistakes and troubleshooting

- **Mistake:** adding side effects in a read-oriented tool.
- **Mistake:** bypassing policy path for convenience.
- **Mistake:** returning unstructured errors that break UX/reporting.
- **Fix:** keep tool outputs structured and route all side effects through permission-checked paths.
