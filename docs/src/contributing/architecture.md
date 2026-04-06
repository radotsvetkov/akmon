# Architecture

## Dependency graph (simplified)

```
akmon-cli
 ├── akmon-config
 ├── akmon-core
 ├── akmon-models
 ├── akmon-query ──┐
 │                 ├── akmon-tools ── (optional index)
 └── akmon-tui ────┴── (bridges UI + query)
```

## Core concepts

| Component | Responsibility |
| --- | --- |
| **Sandbox** (`akmon-core`) | Path allow-lists inside repo root |
| **PolicyEngine** | Allow / deny / prompt per tool action |
| **Agent FSM** | High-level agent state machine |
| **`LlmProvider`** (`akmon-models`) | Streaming completions per vendor |
| **AgentSession** (`akmon-query`) | Tool loop, summarization hooks |
| **TuiApp** (`akmon-tui`) | Transcript, overlays, metrics |

## Tool flow (simplified)

```
User message → session → provider stream
  → tool calls → policy → tool execution → audit JSONL
  → model consumes results → loop → Done
```

## Adding a tool

1. Implement the `Tool` trait in `akmon-tools`.
2. Register it in the tool registry for the modes that should expose it.
3. Add tests + docs references under [Tools](../reference/tools.md).

For providers see [Adding a provider](./providers.md).
