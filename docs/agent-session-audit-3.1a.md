## Pre-Integration Audit Findings (Item 3.1, Part 1/3)

No code changes made. This is a read-only audit.

### A) `AgentSession` structure

- **Current constructor signature (exact):**
```rust
pub fn new(
    config: AgentConfig,
    policy: Arc<akmon_core::PolicyEngine>,
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    sandbox: Arc<Sandbox>,
    akmon_md: Option<String>,
    plan_mode: bool,
) -> Self
```
(from `crates/akmon-query/src/session.rs`)

- **Lifecycle boundaries in code (start -> turns -> end):**
  - **Session object start:** construction at `AgentSession::new(...)` from:
    - `crates/akmon-cli/src/main.rs`
    - `crates/akmon-tui/src/agent.rs`
    - `crates/akmon-query/src/subagent_tool.rs`
  - **Turn start boundary:** `AgentSession::run(...)` begins by calling `prepare_for_new_user_turn()`.
  - **Turn loop boundary:** `'session: loop` in `run`.
  - **Turn end boundary:** all `return Ok(())` / `return Err(...)` exits in `run`, especially around `AgentEvent::Done` branches and error branches.
  - **Session end boundary (object lifetime):** no explicit teardown hook today; session ends when caller drops it (or replaces it in TUI reload path).

- **Where provider calls are made (all places):**
  - Main completion loop: `self.provider.complete(&messages, &completion_config).await` in `run`.
  - Context summarization path: `self.provider.complete(&summary_msgs, &sum_config).await` in `run_context_summarization_pass`.
  - (Separate from loop, already available wrapper infra): `JournalingProvider` in `crates/akmon-models/src/journaling.rs`.

- **Where tool dispatch happens:**
  - `dispatch_tool_calls_batch(...)` in `crates/akmon-query/src/session.rs`.
  - Called from all 3 stop-reason branches where tool calls can appear (`MaxTokens`, `EndTurn`, `ToolUse`) inside `run`.

- **Where permission checks fire:**
  - Centralized in `dispatch_tool_calls_batch(...)`:
    - `evaluate_mcp_automatic(...)`
    - `evaluate_automatic_for_tool(...)`
    - `resolve_interactive(...)`
    - interactive confirmation via `AgentEvent::ConfirmationRequired` + `interactive_policy_rx.recv().await`
  - Includes remembered approvals / allow-all-writes / shell-prefix session shortcuts.

- **Where retrieval happens:**
  - No first-class retrieval phase in the session loop.
  - Retrieval-like behavior currently occurs as regular tool execution (e.g. `semantic_search`, `search`, `read_file`, optional `web_fetch`) via `dispatch_tool_calls_batch(...)`.
  - `EventKind::RetrievalCall` exists in journal schema (`crates/akmon-journal/src/event.rs`) but is not emitted by the loop today.

### B) Coupling surface for adding `JournalHandle { store, graph }` to `AgentSession::new`

- **What breaks immediately:**
  - All `AgentSession::new(...)` call sites must change:
    - Runtime call sites in `crates/akmon-cli/src/main.rs`, `crates/akmon-tui/src/agent.rs`, `crates/akmon-query/src/subagent_tool.rs`.
    - Many test call sites in `crates/akmon-query/src/session.rs` tests.
  - Any helper/factory that constructs sessions (`build_agent_session`, architect/planner branches) must pass handle(s).

- **State currently held by session that is not directly capturable/serializable:**
  - `provider: Arc<dyn LlmProvider>` (trait object, runtime-bound)
  - `tools: Vec<Arc<dyn Tool>>` (trait objects)
  - `policy: Arc<PolicyEngine>` and `sandbox: Arc<Sandbox>` (runtime handles)
  - These are capturable as references/IDs in events, but not serializable session snapshot state by default.
- **Closures/non-serializable caches inside `AgentSession` itself:**
  - No closure fields in `AgentSession`.
  - Major non-serializable surface is trait-object runtime dependencies, not closure capture.

### C) Event emission points vs Appendix A.7

Appendix A.7 variants (from `docs/planning/AKMON_V2_DECISION_DOCUMENT.md`) are:

`SessionStart`, `UserTurn`, `ProviderCall`, `ToolCall`, `RetrievalCall`, `PermissionGate`, `AssistantTurn`, `SessionEnd`.

Recommended insertion points in current loop (instrumentation, not refactor):

1. **`SessionStart`**
   - Emit once at first session boundary before first turn execution (practically: start of first `run(...)` after session construction).
   - Risk: TUI keeps one `AgentSession` across many turns; define whether one journal session spans all turns or each `run` is a session. This is a product/semantics decision.

2. **`UserTurn`**
   - Emit when user task is accepted for a run, right after `prepare_for_new_user_turn()` succeeds in `run(...)`.
   - Hash source: incoming `task`.

3. **`ProviderCall`**
   - Main call: around `self.provider.complete(&messages, &completion_config).await`.
   - Summarization call: around `self.provider.complete(&summary_msgs, &sum_config).await`.
   - Wrapper exists already (`JournalingProvider`), but session wiring determines whether both paths are wrapped.

4. **`ToolCall`**
   - Per approved/dispatched tool in `dispatch_tool_calls_batch(...)`.
   - Natural boundaries:
     - Dispatch event at `AgentEvent::ToolCallDispatched`
     - Completion at `AgentEvent::ToolCallCompleted`
   - Wrapper exists (`JournalingTool`), but currently tool registry wiring does not show it being applied in main CLI/TUI construction paths.

5. **`RetrievalCall`**
   - Hard part: loop does not distinguish retrieval tools as a first-class category.
   - Current viable instrumentation point is inside `dispatch_tool_calls_batch(...)` using tool identity classification (e.g. semantic/search/read-only retrieval tools).
   - This is brittle unless you add explicit retrieval metadata/classification on tools.

6. **`PermissionGate`**
   - Emit where policy decisions are made in `dispatch_tool_calls_batch(...)`:
     - MCP automatic checks
     - automatic-for-tool checks
     - interactive confirmation resolve path
     - remembered/session shortcut allows (still decisions)
   - This is feasible without refactor, but there are multiple decision branches; you need one consistent adapter point.

7. **`AssistantTurn`**
   - Emit when assistant output is committed to session context:
     - branches that push `MessageRole::Assistant` (including with tool call record JSON and pure text end-turn).
   - There are multiple assistant-commit sites in `run(...)`, not one single sink.

8. **`SessionEnd`**
   - Emit on terminal turn exits in `run(...)` (successful `Done`, budget-stop `Done`, and error exits).
   - Hardness: multiple early returns; to guarantee single emission you need disciplined centralized exit instrumentation (or explicit guard pattern). If not centralized carefully, double/missed emit risk.

**Hard-to-emit flags (explicit):**
- `RetrievalCall` is hard to emit correctly with current loop structure because retrieval is not a first-class concept in dispatch.
- `SessionEnd` is easy to miss due to many return paths unless end emission is centralized.
- `PermissionGate` is not blocked by deep closures; decisions are in one method (`dispatch_tool_calls_batch`) but spread across several branches.

### D) Lowest-blast-radius integration sequence (instrument-only)

Per your requested order:

1. **Session boundaries first**
   - Add `SessionStart` and `SessionEnd` emission around `run` outer boundaries.
   - First decision required: one AGEF session per `AgentSession` object vs per `run()` call (especially important for TUI multi-turn lifetime).

2. **Then `UserTurn`**
   - Emit once per `run(task, ...)` immediately after turn prep succeeds.

3. **Then `ProviderCall`**
   - Use provider wrapper path already implemented (`JournalingProvider`) and ensure both normal and summarization provider calls run through it.

4. **Then `ToolCall`**
   - Instrument in `dispatch_tool_calls_batch(...)` (or enable tool wrapper wiring globally via tool registry construction).

5. **Then `RetrievalCall`**
   - Start with conservative classification in dispatch (explicit allowlist of retrieval tools).
   - If classification is not acceptable, this becomes a required design change (tool capability metadata) before safe instrumentation.

6. **Then `PermissionGate` last**
   - Add event emission at each policy decision point in `dispatch_tool_calls_batch(...)`.
   - Keep existing policy behavior untouched; only append journal event records.

### Refactor requirement check

- **No loop refactor is strictly required** to start instrumentation for `SessionStart`, `SessionEnd`, `UserTurn`, `ProviderCall`, `ToolCall`, and `PermissionGate`.
- **Potential design-level change may be required** for robust `RetrievalCall` semantics (first-class retrieval classification) if name-based heuristics are not acceptable.
