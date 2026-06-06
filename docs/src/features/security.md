# Security model

Documented for Akmon `2.2.0`.

Akmon is an evidence and verification layer for AI agents. Its security model has two halves, and they are deliberately separate.

The first half is the verification layer, which is the product. It does not trust the producer. A signed AGEF bundle can be checked offline, by a third party, with nothing but `openssl`. The trust you place in a record comes from a public key you already hold, not from anything the producer asserts about itself.

The second half is the runtime control surface of Akmon's own reference agent. When you run the bundled agent, side effects pass through typed permissions, sandbox boundaries, and policy. Those controls are what let a reference-agent session record an honest `full` capture level. They are not the trust boundary for an imported third-party trace. An OpenTelemetry import records what the trace contained, at `structural` capture level, and the verification layer never pretends a structural import is a full recording.

This page covers both halves. Read it as: what a verifier can rely on without trusting you, then what the reference agent enforces at runtime.

## What the verification layer guarantees

The verification layer makes claims that hold regardless of who produced the session.

- Integrity. AGEF objects are SHA-256 content-addressed and the event chain is hash-linked. Any change to any recorded byte changes the head, and the head is what gets signed. `akmon bundle verify` and the standalone `agef-verify` recompute the chain and reject a tampered bundle.
- Authorship. An optional offline Ed25519 signature (`akmon bundle sign`) covers the session head through the canonical `AGEF-SIG-v1` statement. A verifier checks it against a public key supplied out of band. No network, no Akmon install required for the math: `akmon bundle prove-openssl` emits the exact bytes and the `openssl pkeyutl -verify` command.
- Accountability. An optional operator attestation (`akmon bundle attest`) records a separately signed `AGEF-OPERATOR-v1` claim about the accountable person. Verification attaches trust to the attesting key, never to the self-asserted `operator_id`, `role`, or `org` strings. A name is only as trustworthy as the key that signed it, and that key trust is established out of band.

These properties are what make a session usable as evidence in front of a party who does not trust you and does not run your tools.

### Trust attaches to keys, not to strings

This is the single most important property to internalize. The `operator_id`, `role`, and `org` fields in an attestation are self-asserted strings carried verbatim. The only trust signal is whether a distinct attestation verifies against a key you supplied with `--operator-key`. A session attested with no trusted key on hand reports an unverified status, which is not a failure on its own, just an absence of established trust. You decide which keys to trust, and you do that through your own out-of-band process.

### Offline and producer-independent

The verifier does not have to trust the machine that produced the bundle, the agent that ran, or the operator's claims. A signature check needs three inputs: the signed statement bytes, the detached signature, and a public key the verifier already trusts. `akmon bundle prove-openssl` writes all three to a directory and prints the `openssl` command. Stock OpenSSL 3.x verifies it. This is the floor under every other claim on this page: if the cryptography does not check out with a tool the verifier already trusts, nothing else matters.

## Capture honesty

Akmon never overstates what a record contains.

- A reference-agent session, run by Akmon's own bundled agent, records `full` capture. It captured the events it claims to have captured, and it can be replayed deterministically.
- An OpenTelemetry import (`akmon otel import`) records `structural` capture. It is a faithful transcription of what the trace carried, not a full recording. `akmon bundle verify --require-capture full` fails on it, and `akmon replay` refuses it.

`--require-capture full` is the gate to use when a workflow must reject anything weaker than a full recording. Capture level is part of the record, so a verifier sees the honest level and decides whether it meets their bar.

## The reference agent runtime: side-effect control

The rest of this page concerns Akmon's own reference agent. The risk it manages is not model output text. The risk is model-triggered side effects:

- writing files,
- running shell commands,
- accessing network resources,
- mutating git state.

The reference agent mediates each of these through sandboxing, typed permissions, and policy, and records every decision in the audit chain. That mediation is what earns the `full` capture level. None of it applies to imported third-party traces, which carry only what the producing agent emitted.

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

For automation and CI, file-modifying tools also expose `dry_run` validation:

- run `patch` / `apply_patch` / `edit` / `write_file` with `dry_run: true`,
- inspect the returned `file_change_set` (`mode: "dry_run"`, `summary`, `risk`, per-file `changes`),
- execute the same tool call without `dry_run` only when risk and diffs are acceptable.

## Policy-as-code (`Configured`)

`Configured` policy mode supports declarative allow/deny rules for:

- filesystem read/write paths,
- shell command prefixes,
- network domains,
- tool names,
- MCP server names and MCP tool names.

Evaluation is deterministic: explicit deny wins, and the most specific matching rule is selected within each rule list.

### MCP governance hardening (fail-closed)

MCP tool calls are governed by dedicated policy dimensions:

- `mcp.servers.allow` / `mcp.servers.deny`
- `mcp.tools.allow` / `mcp.tools.deny`

Execution posture is fail-closed:

- missing MCP context (server/tool) denies,
- ambiguous MCP context denies,
- parent policy modes without configured MCP rules deny,
- explicit deny rules win over allow matches.

MCP calls still pass normal permission checks after MCP policy approval. There is no bypass path.

## Enterprise policy profiles and packs

Akmon supports reusable policy governance inputs for org rollout:

- built-in profiles (`dev`, `staging`, `prod`),
- policy packs loaded from `.akmon/policy-packs/*.toml|json`,
- a project-local policy file (`.akmon/policy.toml` or `.akmon/policy.json`),
- an optional CLI override (`--policy-override`).

Precedence is explicit and deterministic:

`profile < packs < project-local < CLI override`

This enables staged hardening from development to production without changing the underlying permission classes. See [Policy profiles and packs](./policy-profiles.md) for the full rollout model.

Recommended posture:

- `dev`: fast local iteration with controlled side effects.
- `staging`: tighter shell/network/tool posture for pre-prod automation.
- `prod`: explicit-deny heavy posture with minimal mutation surface.

The selected profile and pack contents feed the effective policy, and the effective policy is hashed into evidence as `policy_hash`. A governance change is therefore visible to CI even when the behavioral effect is subtle.

## Nested/subagent safety ceiling

`spawn_subagent` runs under a strict parent permission ceiling:

- nested sessions never seed broad "allow all writes" approvals,
- parent interactive mode is downgraded to read-oriented nested execution with no implicit side effects,
- tool eligibility is pre-filtered with policy evaluation using tool-name context,
- side-effect decisions still pass through normal permission checks at dispatch time.

This closes the class of escalation where a nested run could gain broader write/shell/network rights than the parent session posture.

Before:

- nested bootstrap pre-seeded broad interactive allow replies,
- side-effect tools could be available in nested runs even when parent posture was read-only.

After:

- nested runs fail closed when confirmations cannot be satisfied safely,
- nested tool access is a subset of parent policy capability.

## Network and SSRF posture

`web_fetch` applies protections against common private-address and metadata-endpoint abuse patterns. This reduces risk from prompt injection that tries to exfiltrate internal data.

## Secrets handling

Operational guidance:

- keep keys in environment variables or secured config paths,
- never paste production credentials into prompts,
- rotate credentials immediately if leakage is suspected,
- store signing keys (the `.pk8` files from `akmon bundle keygen`) the way you would store any private signing material. Akmon writes them `0600` on unix, but their security after that is your custody process.

## What `--yes` is and is not

`--yes` is a productivity flag, not a blanket "do anything" bypass. It primarily streamlines read-oriented operations. Mutating actions remain policy-gated.

## Reliability metrics are observability only

Run and evidence reliability counters (tool success rates, denials, retries, timeouts) are for operational visibility and SLO monitoring. They do not grant permissions and do not bypass policy enforcement. See [Reliability and SLO metrics](./reliability-slos.md).

## Common mistakes and troubleshooting

- Mistake: treating a `structural` OTEL import as if it were a full recording.
  - Fix: gate strict workflows with `akmon bundle verify --require-capture full`, and use replay only on reference-agent sessions.
- Mistake: trusting an operator name because it appears in a bundle.
  - Fix: trust the attesting key. Verify with `--operator-key` and establish key trust out of band.
- Mistake: enabling broad shell access in unattended workflows.
  - Fix: restrict with precise allow patterns in a `prod` profile or pack.
- Mistake: assuming audit logs replace code review.
  - Fix: use logs plus normal review and CI controls.
- Mistake: storing sensitive logs or signing keys in version control.
  - Fix: keep `.akmon/` artifacts and key material out of source control unless policy requires it.

## See also

- [Audit log](./audit-log.md)
- [Evidence artifact](./evidence.md)
- [Policy profiles and packs](./policy-profiles.md)
- [Reliability and SLO metrics](./reliability-slos.md)
- [Security policy](../security.md)
