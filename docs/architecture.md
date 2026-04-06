# Architecture

## Crate structure

```
akmon-cli
  ├── akmon-config   (user ~/.akmon/config.toml)
  ├── akmon-tui      (interactive terminal UI)
  ├── akmon-index    (optional semantic index)
  └── akmon-query
        ├── akmon-models
        ├── akmon-tools
        │     └── akmon-core
        └── akmon-core
```

**akmon-cli** is solely responsible for the terminal binary: argument parsing, wiring providers (Ollama vs Anthropic), choosing output mode (`text` / `json`), resolving the audit log path, and spawning the async session loop. It must never embed business logic for tools, policy rules, or model protocol details beyond what is needed to construct dependencies.

**akmon-query** owns the **agent session**: the main `run` loop, iteration accounting, message assembly, streaming handling, tool dispatch, and bridging model events to the core FSM. It must never duplicate low-level path checks (those belong in `Sandbox`) or reimplement provider HTTP/SSE (that belongs in `akmon-models`).

**akmon-models** implements **LLM backends** and wire formats: HTTP clients, streaming parsers, and message/tool-call mapping to Akmon’s shared model types. It must never perform filesystem access or policy decisions.

**akmon-tools** defines concrete **tools** (filesystem, optional `web_fetch`, and MCP HTTP proxies in `mcp_client`) and their argument validation, delegating path safety to **akmon-core** for file paths. Network tools declare **`NetworkFetch`** permissions and are evaluated by the policy engine like any other tool.

**akmon-core** holds **shared primitives**: permissions, policy engine, audit events, sandbox resolution, secrets, and FSM types/transition rules. It must not depend on higher crates (no imports from `akmon-query` or `akmon-cli`) and must remain the single source of truth for “what is allowed” and “what was recorded.”

## Agent state machine

```
[Idle]
  → Planning (user submits task)
[Planning]
  → Thinking (model responds)
  → Failed (error, policy, limit)
[Thinking]
  → ToolExecution (tool calls present)
  → AwaitingConfirmation (write / sensitive action)
  → Complete (end turn, no tools)
  → Failed (truncated, error)
[ToolExecution]
  → Thinking (tools completed)
  → Failed (tool error)
[AwaitingConfirmation]
  → Thinking (confirmed or denied)
  → Failed (timeout)
[Summarizing]
  → Thinking (summary done)
  → Failed (summary failed)
[Complete] terminal
[Failed] terminal
```

## Data flow

1. The CLI parses arguments, resolves the working directory, and **detects the git project root** for sandboxing.
2. A **Sandbox** is constructed from that root; all later file paths are validated through it.
3. **AKMON.md** is read from the project root when the file exists and passed into the session as optional context.
4. A **PolicyEngine** is created in the mode implied by flags (for example interactive vs auto-approve reads).
5. **AgentSession::new** receives **AgentConfig** (iteration limit, timeouts, session id), the policy handle, the **LlmProvider**, the tool registry, sandbox, and AKMON content.
6. **run** begins: **check_iteration_limit** guards each model iteration against **max_iterations**.
7. **build_messages** assembles the prompt: delimited **AKMON.md** block (if any), **project context**, prior **history**, and the user **task**.
8. **provider.complete** streams **StreamEvent** values (text deltas, tool calls, completion signals).
9. The session maps stream events to **AgentEvent** values and applies **validate_transition** / **next_state_after** so the FSM stays legal.
10. **TextDelta** events append to **result_text** and, in text mode, print incrementally to the terminal.
11. On **ToolUse**, each call is checked by policy; allowed tools run with **ToolContext**; results are appended as **tool** role messages for the next iteration.
12. The iteration counter advances; **check_iteration_limit** runs again before the next model call.
13. The loop continues until **EndTurn** with no further tool work, success completion, or a hard stop (limit, fatal error, policy denial).
14. **write_audit_jsonl** flushes all **AuditEvent** records for the session to the chosen JSONL path.
15. The CLI prints either streamed text as already shown or a single **RunReport** JSON object on stdout when `--output json` is set.

## Tools

Built-in tools are registered in **akmon-cli** from **akmon-tools** (read/search/edit/patch/write, optional `shell` and `web_fetch`). The CLI may also append **MCP**-discovered tools after `tools/list` against each `--mcp-server` URL.

### MCP tools

Akmon can connect to any MCP (Model Context Protocol) server and automatically register its tools at startup. Use `--mcp-server` to specify server URLs. Multiple servers are supported. Tool discovery uses `tools/list` and tool execution uses `tools/call` via JSON-RPC 2.0 over HTTP.

## Security architecture

**Path confinement** is enforced by **Sandbox::resolve**: candidate paths are joined relative to the project root, **`dunce::canonicalize`** (or equivalent) produces a stable absolute path, and the implementation checks that the resolved path **still lies under** the canonical project root prefix. Doing canonicalization **before** the prefix test closes gaps where symbolic links or normalization might otherwise point outside the tree.

**Secret isolation** uses **`Secret<T>`** in **akmon-core**: the type deliberately **does not implement `Debug`**, payloads are **zeroized on drop**, and the only read API is **`expose_secret()`**, which call sites use briefly (for example when attaching the Anthropic key to an HTTP header). That pattern keeps credentials out of logs and accidental `{:?}` formatting.

**Prompt injection mitigation** relies on **fixed structural delimiters** in message construction: project files and `AKMON.md` are wrapped in labeled blocks, tool outputs are clearly framed, and the model is steered by a stable system preamble so repository content is treated as **untrusted data** rather than as instructions with the same authority as the system prompt.

**Policy enforcement** requires that **every permission-bearing operation** go through the policy engine, which emits an **AuditEvent::PolicyEvaluation** (verdict plus reason) for each decision. Tools do not “skip” policy for convenience; denials surface as structured errors, and the audit log provides a complete trace with **no silent bypass path**.

## Known limitations (v1.3.0)

- Shell tool output is passed as raw text to the model. Numeric results (test counts, line counts) may be interpreted approximately rather than exactly. For precise counts use `--output json` and parse the `tool_calls` field.

- Candle local inference backend is not yet implemented. Local model support requires Ollama.

- Headless `akmon chat` / `akmon run` remain available; interactive sessions use **`akmon`** (TUI) or the same agent loop with streamed terminal output.

- Anthropic prompt caching applies to eligible system content; very large or frequently changing prompts may still incur higher token use—see release notes in `CHANGELOG.md`.
