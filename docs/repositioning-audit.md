# Repositioning Audit (Item 0.2)

This is a read-only audit of the current repository against the v2.0 decision plan.

## A. Current crate structure

### `akmon-core`
`akmon-core` is the shared trust-and-state kernel, not a narrow utility crate. It currently owns sandbox/path validation, permission typing, policy evaluation, FSM transition validation, audit-chain schema/verification, replay metadata hashing, evidence artifact validation, and additional project/profile helpers. In practice, almost every higher crate depends on it for correctness-critical behavior.

### `akmon-config`
`akmon-config` is the user configuration adapter around `~/.akmon/config.toml`: loading/saving, schema defaults, key masking for display, and MCP/policy/SLO config materialization. It does not execute policy decisions itself; it provides configuration inputs.

### `akmon-models`
`akmon-models` defines the `LlmProvider` abstraction and concrete providers (Anthropic, Ollama, OpenAI-compatible, Bedrock), plus shared streaming/event normalization and token estimation hooks. It encapsulates provider-specific HTTP/streaming behavior, including retry/rate-limit handling.

### `akmon-tools`
`akmon-tools` is the tool execution surface: `Tool` trait, `ToolContext`, and built-in tools (filesystem, shell, web fetch, git, MCP, semantic search, todos/spec tools). It standardizes outputs through `ToolOutput` and error codes for session-level orchestration.

### `akmon-query`
`akmon-query` contains the orchestrator (`AgentSession`) and is the operational control plane. It owns run lifecycle, context assembly/compaction, provider invocation, tool dispatch, permission integration, FSM event application, and in-memory audit accumulation.

### `akmon-index`
`akmon-index` is the semantic retrieval subsystem: chunk/index types, vector search ranking, and persisted index storage. It is feature-gated and consumed by tools/TUI/CLI when semantic indexing is enabled.

### `akmon-tui`
`akmon-tui` is the interactive terminal runtime layered over `akmon-query`. It handles rendering, user interaction, slash flows, and session persistence concerns, while delegating core orchestration to `AgentSession`.

### `akmon-cli`
`akmon-cli` is the composition root and command surface. It wires policy mode, sandbox, provider, tools, TUI/headless execution, and final artifact writes (audit/evidence/session reports).

### Cross-crate dependency risks
- No Cargo-level circular dependencies were found in workspace manifests.
- Layering is mostly clean (`core` -> `models/tools/index/query` -> `cli/tui`).
- Notable architectural risk: `akmon-core` has broad ownership across unrelated concerns (policy/FSM/audit/replay/evidence/project profiling), i.e., a potential "god core" trajectory.

## B. Agent loop reality

`AgentSession` starts via `AgentSession::new(...)`, and `run(...)` immediately prepares per-turn state (`prepare_for_new_user_turn`) and enforces an `Idle` precondition before entering the main `'session` loop. Turn progression is event-driven rather than explicit persisted commits: boundaries are defined by `StopReason` handling (`EndTurn`, `ToolUse`, `MaxTokens`) plus iteration checks and context appends. Provider calls are concentrated in two places: the main completion call in `run(...)` and a separate summarization call in `run_context_summarization_pass(...)`. Tool calls are centralized in `dispatch_tool_calls_batch(...)`, including pre-dispatch policy resolution, approval filtering, parallel execution, ordered result reintegration, and context append of tool outputs.

Permission checks are integrated before tool execution by deriving concrete permissions (`concrete_permissions(...)`) and evaluating policy (`evaluate_automatic_for_tool(...)` / `resolve_interactive(...)`), with session memory for approvals (`remember_for_session`, write-all, shell-prefix allow). Retry/continuation behavior exists in multiple layers: session-level continuation loops for truncation and provider-level rate-limit/backoff logic inside model backends. This means repeated attempts can legitimately emit additional events/messages; any v2 capture design must define attempt semantics clearly to avoid ambiguous double-emission interpretation.

## C. Audit event reality

Current audit captures four event families in `akmon-core`: `PolicyEvaluation`, `ToolDispatch`, `ToolOutcome`, and `AgentStep`. Events are stored in-memory during session execution and serialized as hash-chained JSONL (`AuditChainRecord`) using SHA-256 over canonicalized JSON (`prev_hash + event`), with schema marker `audit_chain.v1` and terminal `session_final_hash` on the last record. Audit writing is performed by CLI/TUI orchestration (`write_audit_jsonl(...)`) after run completion paths.

Compared to Appendix A.7 EventKind:
- **Direct overlap**: policy gating intent (`PolicyEvaluation` ~ `PermissionGate`), and tool execution lifecycle (`ToolDispatch`/`ToolOutcome` ~ `ToolCall` facets).
- **Partial/absent coverage**: no first-class typed events for `SessionStart`, `UserTurn`, `AssistantTurn`, canonical `ProviderCall` request/response/stream object references, or `RetrievalCall`.
- **Unification/retirement candidates**:
  - `AgentStep` (broad string-description event) should likely be replaced by typed event kinds in the substrate graph.
  - split `ToolDispatch` + `ToolOutcome` can be normalized under a typed call model with object references.
  - policy rows should converge toward a single `PermissionGate` event shape.

## D. Assumptions that are wrong (or need refinement)

### `LlmProvider` wrappability
The assumption is mostly valid. `LlmProvider` is object-safe and already consumed as `Arc<dyn LlmProvider>`, with one async completion method returning a stream abstraction. Wrapping is feasible with low trait friction. The main nuance is behavioral, not type-level: provider backends already implement retry/backoff logic internally, so wrapper capture must define whether it records final outcome only or full attempt history.

### `Tool` wrappability
This assumption is also mostly valid. `Tool` is object-safe and used dynamically. However, permission logic is not fully encapsulated in the tool trait: session-level `concrete_permissions(...)` contains tool-name-specific permission expansion rules. Therefore, a pure `Tool` wrapper can capture tool I/O cleanly, but permission events remain session-level concerns.

### Retrieval canonical hashability
`akmon-index` data structures are serializable, so canonical hashing is possible in principle. But retrieval outputs currently include floating similarity scores and generated preview text in semantic search responses. Deterministic hashing is still achievable, but requires an explicit canonicalization contract (field ordering, float normalization policy, and what is considered normative vs presentation-only).

## E. Blocker candidates (rethinking risk, not just effort)

- **Substrate mismatch (major):** current audit is sidecar JSONL written after run paths, not an append-only content-addressed object store with a Merkle session graph. Delivering P0 substrate goals requires introducing a new durability model, not only extending existing audit serialization.
- **Full-capture gap (major):** current system does not persist a unified object graph of prompts, provider request/response streams, tool I/O, retrieval artifacts, and permission decisions as first-class content-addressed objects.
- **Event-model mismatch (major):** current event taxonomy is operational/log-oriented (`AgentStep` especially), while v2 Appendix A expects typed, portable event kinds with stable replay/verification semantics.
- **Architecture hygiene risk (moderate):** `akmon-core` breadth can increase migration risk if substrate work is added without boundary discipline; this is not an immediate blocker but is a planning-critical concern before Phase 1.

---

References inspected include: `crates/*/Cargo.toml`, `crates/akmon-query/src/session.rs`, `crates/akmon-models/src/lib.rs`, `crates/akmon-tools/src/lib.rs`, `crates/akmon-index/src/{lib.rs,search.rs,persist.rs}`, `crates/akmon-core/src/{audit.rs,replay.rs,evidence.rs,lib.rs}`, and `crates/akmon-cli/src/main.rs`.
