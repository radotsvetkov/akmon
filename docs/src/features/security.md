# Security model

Akmon treats side-effect control as a core system, not a UI option.

## Threat model in plain terms

The main risk is not "model output text." The risk is model-triggered side effects:

- writing files,
- running shell commands,
- accessing network resources,
- mutating git state.

Akmon addresses this with sandboxing, typed permissions, and audit logs.

## Sandbox boundaries

File operations are constrained to project boundaries. Path traversal attempts are blocked. This prevents prompt-driven writes to unrelated filesystem locations in normal operation.

## Permission classes

| Class | Typical actions | Default posture |
| --- | --- | --- |
| Read | list/read/search | easier to auto-approve (`--yes`) |
| Write | write/edit/patch | requires explicit confirmation/policy allow |
| Shell | command execution | allowlisted/confirmed paths |
| Network | web fetch/MCP-backed actions | policy-checked and traceable |
| Git mutating | add/commit/restore/etc. | confirmed or explicitly policy-approved |

## Diff-first approvals

For file changes, Akmon can present unified diffs before final approval. This gives human review at the moment side effects happen, not only at the end.

## Policy-as-code (`Configured`)

`Configured` policy mode supports declarative allow/deny rules for:

- filesystem read/write paths,
- shell command prefixes,
- network domains,
- tool names.
- MCP server names and MCP tool names.

Evaluation is deterministic: explicit deny wins, and the most specific matching
rule is selected within each rule list.

### MCP governance hardening (fail-closed)

MCP tool calls are now governed by dedicated policy dimensions:

- `mcp.servers.allow` / `mcp.servers.deny`
- `mcp.tools.allow` / `mcp.tools.deny`

Execution posture is fail-closed:

- missing MCP context (server/tool) denies,
- ambiguous MCP context denies,
- parent policy modes without configured MCP rules deny,
- explicit deny rules win over allow matches.

MCP calls still pass normal permission checks after MCP policy approval (no bypass path).

## Enterprise policy profiles and packs

Akmon supports reusable policy governance inputs for org rollout:

- built-in profiles (`dev`, `staging`, `prod`),
- policy packs loaded from `.akmon/policy-packs/*.toml|json`,
- project-local policy file (`.akmon/policy.toml` or `.akmon/policy.json`),
- optional CLI override (`--policy-override`).

Precedence is explicit and deterministic:

`profile < packs < project-local < CLI override`

This enables staged hardening from development to production without changing the underlying permission classes.

Recommended posture:

- `dev`: fast local iteration with controlled side effects.
- `staging`: tighter shell/network/tool posture for pre-prod automation.
- `prod`: explicit-deny heavy posture with minimal mutation surface.

## Nested/subagent safety ceiling

`spawn_subagent` now runs under a strict parent permission ceiling:

- nested sessions never seed broad "allow all writes" approvals,
- parent interactive mode is downgraded to read-oriented nested execution (no implicit side effects),
- tool eligibility is pre-filtered with policy evaluation using tool-name context,
- side-effect decisions still pass through normal permission checks at dispatch time.

This closes the class of escalation where a nested run could gain broader write/shell/network
rights than the parent session posture.

Before:

- nested bootstrap pre-seeded broad interactive allow replies,
- side-effect tools could be available in nested runs even when parent posture was read-only.

After:

- nested runs fail closed when confirmations cannot be satisfied safely,
- nested tool access is a subset of parent policy capability.

## Network and SSRF posture

`web_fetch` applies protections against common private-address and metadata endpoint abuse patterns. This reduces risk from prompt injection that tries to exfiltrate internal data.

## Secrets handling

Operational guidance:

- keep keys in environment or secured config paths,
- never paste production credentials into prompts,
- rotate credentials immediately if leakage is suspected.

## What `--yes` is and is not

`--yes` is a productivity flag, not a blanket "do anything" bypass. It primarily streamlines read-oriented operations; mutating actions remain policy-gated.

## Reliability metrics are observability only

Run/evidence reliability counters (tool success rates, denials, retries, timeouts)
are for operational visibility and SLO monitoring. They do not grant permissions and
do not bypass policy enforcement.

## Common mistakes and troubleshooting

- **Mistake:** enabling broad shell access in unattended workflows.
  - **Fix:** restrict with precise allow patterns.
- **Mistake:** assuming audit logs replace code review.
  - **Fix:** use logs plus normal review/CI controls.
- **Mistake:** storing sensitive logs in version control.
  - **Fix:** keep `.akmon/` artifacts out of source control unless required.
