# Akmon v2.0 — Decision Document
 
**Purpose:** This is the single source of truth for the Akmon v2.0 repositioning. It exists because positioning decisions made in conversation get forgotten; decisions written down stay decided. Everything in this document that is marked **LOCKED** is not up for renegotiation without an explicit revision of this document. Everything marked **OPEN** is awaiting your product-owner decision. Everything marked **BACKLOG** is future work you and Cursor collaborate on incrementally.
 
**How to use this document:**
- **Layer 1** (§1–§4) is positioning. Read once. Reference always. Do not mutate without a new version of this document.
- **Layer 2** (§5) is technical decisions that need your call before code changes. Work through them with Cursor in conversation. Record answers inline.
- **Layer 3** (§6) is sequenced work. Each entry is sized for one Cursor Composer session. Written in your existing Cursor style — Cursor surfaces findings, waits for your approval, paces the work.
- **Appendix A** is the AGEF format spec seed — short, publishable, separate from Akmon's release cycle.
**What this document is not:**
- Not a prompt to paste into Cursor. It's a planning document. Cursor reads it as context; your existing Cursor system prompt drives the interaction style.
- Not a timeline. Estimates are rough (8–10 focused weeks, but you set pace).
- Not a marketing plan. Positioning only. Marketing is downstream of shipped code.
**Document version:** 1.2 — May 2026
**Revision history:**
- v1.0 (April 2026) — Initial document.
- v1.1 (April 2026) — Adds D-16, D-17, Item 6.10 in response to repositioning audit findings A and B (`docs/repositioning-audit.md`). No prior decisions altered.
- v1.2 (May 2026) — Adds Item 4.3 design decisions (F1-F12), renames bundle commands to `akmon bundle ...`, and corrects D-02 manifest serialization wording to align with AGEF v0.1.1 §6.
---
 
# Layer 1 — Locked positioning
 
## §1 Thesis
 
**LOCKED.**
 
> Akmon is the review-aware AI coding agent for regulated engineering, whose every session is a tamper-evident, content-addressed, replayable artifact — not a side effect of a conversation, but the central deliverable.
 
### §1.1 What the thesis commits to
 
- **Review-aware, not review-only.** Akmon supports the spectrum of autonomy (read-only, propose-only, supervised, full) via the existing policy engine. The thesis does not mandate a review gate; it mandates that every action — whatever its autonomy level — produces a first-class, verifiable session artifact.
- **Regulated engineering primary.** Features and decisions are evaluated first against "does this help a developer at a regulated European company use AI on their regulated codebase safely and defensibly." Secondarily against US-regulated contexts (HIPAA, SOC 2). Generalist developer productivity is a tertiary concern.
- **Session as deliverable.** The session artifact is what Akmon ships to the user. The code change is a field inside the session. This inverts the usual agent model (conversation → maybe a diff → audit log as sidecar). Here the artifact is primary.
- **Tamper-evident by construction.** Content addressing and merkle linkage are not features; they are the storage model. You cannot use Akmon without producing a cryptographically verifiable session record, because there is no other way for Akmon to store what happened.
### §1.2 What the thesis explicitly does not commit to
 
- Not "autonomous agent of everything." Akmon is a strong review-aware agent for regulated code, not a replacement for Cursor in greenfield consumer work.
- Not "air-gapped by default." Air-gapped deployment is supported via local inference; it is not the primary story.
- Not "best-in-class completion UX." Cursor wins there. Akmon wins on everything that happens after the completion: review, audit, reproduction, comparison.
- Not "multi-agent orchestration." One agent, one session, one artifact.
---
 
## §2 Personas
 
**LOCKED.**
 
### §2.1 Primary persona — The Regulated Developer
 
A senior developer (5+ years) at a regulated company. EU primary, global regulated secondary. Works in financial services, healthcare, govtech, energy, critical infrastructure, or regulated industrials. Currently uses Cursor or Copilot for greenfield and personal velocity. **Cannot** use them for regulated repositories because their security team has banned cloud-connected AI tools for those codebases, or because data residency or audit requirements forbid it.
 
Their current workaround is one of: using no AI on the regulated repo, using AI on a separate machine and manually porting code, or using AI against obfuscated snippets of the code. All three are friction. All three leave productivity on the table. All three create shadow-IT risk their security team would object to if they knew.
 
They would adopt Akmon because it lets them use AI on the regulated codebase **and** generates the evidence their security team needs to approve the usage. The pitch is not "Akmon is safer than Cursor"; it is "Akmon is the AI agent your security team will let you use on this code."
 
### §2.2 Secondary persona — The Platform/SRE Engineer
 
A platform, SRE, or internal-tools engineer at a mid-size European company who has been tasked by their CTO or VP Engineering to "figure out a policy for AI tooling." They've been told the answer cannot be just "ban it" (engineers are already sneaking it in) and cannot be just "yes" (security won't sign off). They're looking for a third option they can recommend upward.
 
Their adoption is evaluative first, procurement second. They test Akmon on one regulated repo, run it past security, iterate. If it works, they write an internal blog post, they present at brown-bag sessions, they become the pattern others in the company follow. **This persona is how Akmon crosses from personal adoption to organizational adoption.** Treat them as the contributor-in-waiting.
 
### §2.3 Non-personas (explicit exclusions)
 
Akmon is **not** for:
- Solo developers on non-regulated hobby projects. Cursor is better.
- Large engineering orgs needing fleet management of 500 developers' AI usage. That's a different product.
- Enterprise procurement committees buying top-down. They don't exist as a buyer in this plan. If they ever become a buyer, that's a different product sold above Akmon.
- Air-gapped defense contractors with zero-connectivity requirements. Supported as a deployment mode; not the primary story.
---
 
## §3 Feature priorities
 
**LOCKED.**
 
### §3.1 P0 — Without these, no v2.0
 
| # | Requirement | Why |
|---|---|---|
| P0-1 | Policy + sandbox + MCP governance — fail-closed, explainable, existing Akmon strength preserved and hardened | Foundation; security's lever |
| P0-2 | Content-addressed object store + merkle session graph | The substrate everything else depends on |
| P0-3 | Full capture — prompts, model responses (incl. streaming chunks), tool I/O, retrieval results, permission decisions — all hashed into the store | The "what happened" evidence |
| P0-4 | Tamper-evident verification — `akmon verify <session-id>` on the on-disk journal proves chain integrity, object closure, and byte-level object integrity (AGEF Section 13 step 5); portable head-based checks ship with bundle import/export (Item 4.3, manifest carries `head` and session id) | What makes evidence defensible |
| P0-5 | Session inspection — `akmon inspect <session-id>` reads one on-disk journal session by UUID for human and CI consumption (`--format json`), with optional content resolution for referenced object hashes | Required for review workflows |
| P0-6 | Portable bundle — `akmon bundle export <session-id>` produces a self-contained artifact; `akmon bundle import` round-trips | How sessions leave the producer's machine |
| P0-7 | AGEF spec v0.1 published as separate repo | Makes the format a public artifact, not a private detail |
| P0-8 | CI automation — all verify/inspect/export operations produce JSON with documented exit codes | Laptop + CI parity is a user commitment |
 
### §3.2 P1 — Core story of v2.0
 
| # | Requirement | Why |
|---|---|---|
| P1-1 | Replay engine — `akmon replay <head>` with strict / regenerate / dry modes | Makes reproducibility real, not theoretical |
| P1-2 | Session diff — `akmon diff <head_a> <head_b>` with text, JSON, and self-contained HTML output | User-requested; core to the positioning |
 
### §3.3 P2 — Ship with v2.0 if bandwidth allows, else v2.1
 
| # | Requirement | Why |
|---|---|---|
| P2-1 | Bisect — `akmon bisect` across a sequence of sessions | Demo-sexy, community-shareable, but not load-bearing |
| P2-2 | TUI views for timeline and diff | Nice for interactive users; not a differentiator on its own |
 
### §3.4 P3 — Deferred
 
| # | Deferred | Why |
|---|---|---|
| P3-1 | Air-gapped local-inference hardening beyond what exists | Testers not asking; keep running in CI so it doesn't rot |
| P3-2 | SIEM integrations (Splunk HEC, Elastic) | Value-add once AGEF exists; someone can build as ecosystem |
| P3-3 | Approval-workflow UX (propose-diff-accept specifically) | One-tester signal; policy engine already covers the mechanism; revisit when 3+ testers ask |
| P3-4 | Multi-session orchestration | Explicit non-goal |
| P3-5 | Any hosted/SaaS Akmon | Explicit non-goal for v2.0 |
 
### §3.5 Notes on priority
 
1. **AGEF spec is in P0.** Publishing the spec early creates a public commitment that shapes internal engineering. The spec repo exists before the implementation is fully done.
2. **Replay and diff are P1, not deferred.** v2.0 ships the full substrate story, not a teaser. A half-delivered positioning gets a shrug; a full one gets shared.
3. **Decisions D-16 and D-17 (added in v1.1) refine implementation approach without changing feature priorities.** D-16 concerns *how* the journal substrate is introduced (additive, no refactor of existing core crate); D-17 concerns *what* a ProviderCall event captures (full attempt history). Both decisions sit inside the P0 substrate scope; neither expands or contracts what ships in v2.0.
---
 
## §4 v2.0 shippable scope
 
**LOCKED.**
 
Akmon v2.0 is shippable when all P0 items are complete, both P1 items are complete, the akmon-core cleanup pass (Item 6.10) is complete, and the following are true:
 
1. A new user can run `akmon chat` against their own regulated repo, produce a session, verify its integrity, inspect it, export it to a bundle, ship the bundle to a colleague, and have the colleague import it and replay it — all without network access for anything except the model API calls the session itself made.
2. The AGEF spec v0.1 is published in a separate repository under `radotsvetkov/agef`, referenced from Akmon's README.
3. The README reflects the positioning in §1–§2.
4. At least one quoted case study appears in the launch announcement.
5. CI matrix includes a local-inference (Ollama) run and an API-inference (Anthropic or OpenAI) run so air-gap-adjacent capability doesn't silently rot.
v2.0 is **not** blocked on P2 or P3 items. A v2.0 release candidate that ships P0 + P1 + Item 6.10 is acceptable. P2 items can follow as v2.0.x patches or v2.1.
 
---
 
# Layer 2 — Technical decisions
 
These decisions were settled in product-owner conversation and are now LOCKED. Each carries the original tradeoff context to preserve the reasoning.
 
## §5 Decisions
 
### §5.1 Decision D-01: Object store backend — **LOCKED: redb**
 
**Context.** The content-addressed object store needs a durable, embedded, single-writer, Rust-friendly KV backing. Considered: sled, redb, fjall, rocksdb.
 
**Decision.** **redb.** Pure Rust, actively maintained, ACID, stable 2.x. Sled's maintenance has slowed; redb's activity matters for a project whose selling point is integrity. Users reviewing the code will check for healthy dependencies.
 
### §5.2 Decision D-02: Serialization for canonical hashing — **LOCKED: postcard internal, CBOR for hashed payloads, JSON for manifest metadata**
 
**Context.** Every hashed artifact needs a canonical byte representation. Considered: bincode 1.x/2.x, postcard, CBOR, custom.
 
**Decision.** **postcard for Event serialization inside SessionGraph (fast, small, Rust-native). Canonical CBOR for hashed/referenced AGEF payloads (`events.bin` records and hash-addressed object references). JSON for human/auditor-readable manifest metadata (`manifest.json`) per AGEF §6.** The manifest is metadata and not part of the event hash chain; integrity-critical linkage remains in canonical-CBOR event hashing and content-addressed object bytes.
 
### §5.3 Decision D-03: Hash algorithm — **LOCKED: SHA-256 default, BLAKE3 supported**
 
**Context.** Considered: BLAKE3, SHA-256, SHA-512. Compliance auditors and SIEM operators are trained on SHA-256 and recognize it instantly.
 
**Decision.** **SHA-256 default.** BLAKE3 supported via configuration. AGEF spec allows both via the manifest's `hash_algorithm` field; defaults to `"sha256"`. Performance difference (BLAKE3 is 2-5x faster) is irrelevant for Akmon's write volumes; auditor recognizability matters.
 
### §5.4 Decision D-04: Journal location — **LOCKED: per-user default, per-repo opt-in**
 
**Context.** Where does the journal live? Per-repo, per-user, hybrid.
 
**Decision.** **Per-user default** (`$XDG_STATE_HOME/akmon/journal` on Linux/macOS, `%LOCALAPPDATA%\akmon\journal` on Windows). Per-repo opt-in via config (`<repo>/.akmon/journal`) for users who want sessions to follow code via git or for cross-machine sync.
 
### §5.5 Decision D-05: Signing — **LOCKED: plugin hooks for v2.0, native signing v2.1**
 
**Context.** Tamper-evidence via merkle hashing is inherent. Cryptographic signing of session heads is separate.
 
**Decision.** **v2.0 ships with tamper-evidence (hash chain) and plugin hooks** — a configurable post-session command that runs after each `SessionEnd` with the head hash as an argument. Document how to wire it to cosign or GPG. Native signing comes in v2.1 once usage patterns are visible. This avoids key-management ratholes in the v2.0 critical path.
 
### §5.6 Decision D-06: Streaming capture — **LOCKED: per-chunk objects**
 
**Context.** Model responses stream chunks. Per-chunk objects vs. concatenated streams with offsets.
 
**Decision.** **Per-chunk for v2.0.** Each SSE chunk is a content-addressed blob; stream artifact is a `Vec<Hash>` plus per-chunk timing metadata if D-17 attempts contain stream timing. Simplicity wins. If storage bloat becomes real, application-level zstd compression on blobs above 4KB is trivial to add without format change.
 
### §5.7 Decision D-07: Redaction — **LOCKED: post-hoc derivative bundle via `akmon redact`**
 
**Context.** Sessions capture full prompts and tool I/O. Pre-capture redaction breaks reproducibility. Post-hoc preserves it.
 
**Decision.** **Post-hoc redaction via derivative bundle (R3).** Full capture in the raw store (lives on user's machine, under their control). `akmon redact` reads a journal session and writes a new sanitized derivative `.akmon` bundle for sharing; it does **not** mutate journal events or object bytes in place. Explicit documentation that the raw store must be treated as sensitive (same class as source code). Two-tier encrypted storage and vault-style reversible access are deferred to v2.1+ if asked.
 
### §5.8 Decision D-08: AGEF spec governance — **LOCKED: benevolent dictator for v0.1**
 
**Context.** Spec governance options: benevolent dictator, core maintainers, RFC process.
 
**Decision.** **Benevolent dictator for v0.1, documented intent to transition to core maintainers by v1.0.** The spec repo's `GOVERNANCE.md` explicitly says "governance currently informal, see roadmap to formalize by v1.0." Honest, doesn't overclaim, leaves room to grow.
 
### §5.9 Decision D-09: Replay determinism contract — **LOCKED: best-effort with divergence report**
 
**Context.** Strict replay requires model determinism support. Many local backends don't have it.
 
**Decision.** **Best-effort with explicit divergence report.** Strict mode attempts byte-identical and returns a categorized report: "model non-determinism," "tool non-determinism," "timestamp," etc. Users learn what's real. Don't overclaim guarantees that depend on provider features that may not exist.
 
### §5.10 Decision D-10: Diff algorithm — **LOCKED: greedy same-kind-next**
 
**Context.** Pairing algorithm for SessionDiff: greedy, LCS, hash-aware.
 
**Decision.** **Greedy same-kind-next for v2.0.** Fast, predictable. Hash-aware pairing kept as an obvious extension point for v2.1 if usage demands. LCS not built unless asked.
 
### §5.11 Decision D-11: Bundle format — **LOCKED: tar.zst**
 
**Context.** Bundle format options: tar.zst, zip, sqlite, custom.
 
**Decision.** **tar.zst.** Boring, portable, compresses well, inspection tools universally available. AGEF spec mandates this for v0.1 bundle interoperability.
 
### §5.12 Decision D-12: CLI output for CI — **LOCKED: explicit `--format json`**
 
**Context.** Auto-detect TTY vs. explicit format flag.
 
**Decision.** **Explicit `--format json` on every command. Default human-readable. Never auto-detect.** Auto-detect creates invisible behavior differences between CI and local. Explicit is boring and correct. Every command that produces machine-readable output supports `--format json` uniformly.
 
### §5.13 Decision D-13: Logging vs. journal separation — **LOCKED: separate**
 
**Context.** Should `tracing` logs be journal events?
 
**Decision.** **Sidecar.** `tracing` logs are debugging aid for Akmon developers. Journal events are user-facing deliverables. Conflating them makes the journal noisier and less legible. Document clearly that journal is not debug logging.
 
### §5.14 Decision D-14: Contributor agreement — **LOCKED: DCO**
 
**Context.** CLA vs. DCO for external contributors.
 
**Decision.** **DCO.** Lightweight (`Signed-off-by` line), community-friendly, used by Linux kernel, Docker, Goose. CLA signals "we might commercialize someday and reserve the right" — wrong tone for Akmon's positioning. Add via the DCO GitHub App before first external contributor.
 
### §5.15 Decision D-15: Release cadence — **LOCKED: SemVer**
 
**Context.** v2.0 is a major rewrite. Cadence after?
 
**Decision.** **Semantic versioning. v2.0.x for patches, v2.1.0 for P2 features (bisect, TUI), v2.2 for AGEF v0.2 if needed, v3.0 only for another positioning-level change.** Quarterly-ish minor releases. Patch releases as needed. Stability commitment: AGEF v0.1 bundles produced by Akmon v2.0.x will be readable by Akmon v2.x for the entire v2.x line.
 
### §5.16 Decision D-16: akmon-core handling during journal substrate addition — **LOCKED: Path A (additive only)**
 
*Added in v1.1 in response to repositioning audit finding A.*
 
**Context.** The repositioning audit (`docs/repositioning-audit.md`, finding A) identified that `akmon-core` has accumulated broad responsibility — policy, FSM, sandbox, audit chain, replay metadata, evidence validation, project utilities — making it a "god core" with overlap between its existing audit/replay/evidence code and the new journal substrate planned for Phase 1. Two paths existed:
 
- **Path A — additive only.** Phase 1 introduces `akmon-journal` as a new crate. `akmon-core`'s existing audit, replay, and evidence code is left untouched. The two systems coexist for the duration of v2.0 work. A deletion pass before tagging v2.0 retires the redundant `akmon-core` code, replacing it with thin views over `akmon-journal` for backward compatibility.
- **Path B — refactor during Phase 1.** Phase 1 introduces `akmon-journal` *and* surgically moves the audit/replay/evidence responsibilities out of `akmon-core` into the new crate. `akmon-core` is left holding only policy, FSM, and sandbox.
**Decision.** **Path A.** Rationale:
 
1. The existing hash-chained audit in `akmon-core` is an evolutionary precursor to the new substrate, not foreign. Leaving it in place during the substrate build allows verification and comparison against the new system as it comes online.
2. The deletion pass before v2.0 tagging is added as a new backlog item (Item 6.10) so the cleanup is not lost.
3. The risk profile of Phase 1 is reduced: one new crate, no refactor of existing critical-path code. Matches the "additive before subtractive" principle that has guided this plan throughout.
**Implication.** During v2.0 development, two audit systems coexist:
 
- The legacy `akmon-core` audit chain (hash-linked JSONL, written post-run) — unchanged.
- The new `akmon-journal` substrate (content-addressed, merkle-linked, write-during-execution) — added incrementally.
Item 6.10 retires the legacy code and replaces it with a thin renderer that produces the legacy JSONL format from the new substrate, preserving any external tooling that consumes it.
 
### §5.17 Decision D-17: Provider retry capture in ProviderCall events — **LOCKED: Option 2 (full attempt history in one event)**
 
*Added in v1.1 in response to repositioning audit finding B.*
 
**Context.** The repositioning audit (finding B) identified that retry and continuation behavior happens in two layers:
 
- Backend implementations (`akmon-models/src/anthropic.rs`, `openai_compat.rs`, `ollama.rs`) perform rate-limit retries internally, with sleep + backoff, before returning to the caller.
- Session-level retries on truncation (`StopReason::MaxTokens`) and "truncated mid-tool, resuming" branches in `akmon-query/src/session.rs` re-issue completions to continue beyond model output limits.
Without explicit handling, the journaling provider wrapper would only see the final successful attempt, not the retry history. For the regulated-engineering positioning (§1, §2), this is a material loss: an auditor reviewing a session needs to see when the model was rate-limited, when retries occurred, and how long the session actually took including retries. Three options were considered:
 
- **Option 1: Final attempt only.** ProviderCall captures one request_hash and one response_hash. Retry history lost. Simple.
- **Option 2: Full attempt history in one event.** ProviderCall event has a field `attempts: Vec<AttemptRecord>` where each AttemptRecord captures timestamp, status, request_hash, response_hash, and error (if any). The "final" response is the last successful attempt. One event per logical call; multiple attempts inside.
- **Option 3: Each attempt as a nested event.** Multiple ProviderAttempt events under a parent ProviderCall, modeled as graph nodes with parent-child linkage.
**Decision.** **Option 2.** Rationale:
 
1. Captures the auditor-relevant information that Option 1 loses.
2. Preserves a clean one-call-one-event model that makes the graph readable.
3. Compatible with downstream replay and diff: comparing two sessions where one had retries and one did not produces a meaningful diff (the `attempts` arrays differ).
**Schema for AttemptRecord:**
 
```
AttemptRecord {
  attempt_number: u32,            // 1-indexed
  started_at: timestamp,
  ended_at: timestamp,
  status: AttemptStatus,           // Success, RateLimited, NetworkError, ServerError, Other(String)
  request_hash: Hash,              // each retry may have a slightly different request body
  response_hash: Option<Hash>,     // None if attempt failed before producing a response
  stream_hash: Option<Hash>,       // populated if streaming
  error_message: Option<String>,   // human-readable; bytes also captured if produced
}
```
 
**Provider boundary.** The journaling provider wrapper is the boundary at which retries are captured. The audit found backend-internal retries that are invisible to current callers. Phase 2 implementation MUST address this. Three sub-options exist for how:
 
- **2a:** Modify each backend to surface attempt information via a new trait method or callback.
- **2b:** Move retry logic out of backends into the journaling wrapper itself, so each attempt is naturally observable.
- **2c:** Wrap at a level above the current LlmProvider trait — keep current backend behavior; add a higher-level trait that the wrapper implements and that the session uses.
This sub-decision is deferred to Phase 2 design (Item 2.1's design step). The choice between 2a/2b/2c happens during the design conversation in Prompt 2.1, where the actual code shapes are visible.
 
**Continuation retries (`StopReason::MaxTokens`).** Session-level continuations are a *different* mechanism from rate-limit retries. They are user-relevant but do not belong in the `attempts` array — they are logically distinct provider calls that produce additional content. Each continuation produces its own ProviderCall event in the session graph. This is the existing model; no change required.
 
---
 
# Layer 3 — Sequenced work backlog
 
This is the implementation sequence. Each item has: **title**, **goal**, **when to start**, **when it's done**, and **notes for Cursor** — context Cursor should carry into the conversation when working that item.
 
Order matters but isn't strict. Items can be reordered with reason; ordering changes should be recorded as document revisions, not made silently.
 
## §6 Backlog
 
### §6.1 Pre-work (before Phase 1)
 
**Item 0.1 — Adopt Cursor rules for this repositioning**
 
Goal: `.cursor/rules/*.mdc` files exist committing Cursor to the thesis, substrate rules, scope discipline, and existing safety invariants.
 
Status: **completed** in `.cursor/rules/` with files `00-working-style.mdc`, `01-thesis-review-aware-regulated.mdc`, `02-substrate-invariants.mdc`, `03-rust-style-discipline.mdc`, `04-v2-scope-discipline.mdc`, `99-non-negotiable.mdc`.
 
---
 
**Item 0.2 — Audit current repo structure against this document**
 
Goal: Read-only audit producing `docs/repositioning-audit.md` listing crate structure, agent loop reality, audit event reality, wrong-assumption checks, blocker candidates.
 
Status: **completed**. Findings A and B drove decisions D-16 and D-17 in this document.
 
---
 
### §6.2 Phase 1 — AGEF spec seed
 
**Item 1.1 — Create the AGEF specification repository**
 
Goal: A new public repository `radotsvetkov/agef` exists containing `SPEC.md` at v0.1 (see Appendix A), `GOVERNANCE.md`, `README.md`, `examples/`, Apache-2.0 license for code, CC BY 4.0 for SPEC.md.
 
When to start: immediately.
 
When done: public repo exists, linked from Akmon README.
 
Notes for Cursor: Documentation work, not code. Don't over-engineer. Start from Appendix A and expand.
 
---
 
**Item 1.2 — Journal substrate crate**
 
Goal: New crate `akmon-journal` exists, implementing the content-addressed object store and merkle session graph per the AGEF spec. Per D-16, this is **additive only** — no modifications to `akmon-core`.
 
When to start: after Item 1.1 begun (can run parallel).
 
When done:
- Crate compiles clean.
- Unit and integration tests pass.
- `cargo test -p akmon-journal` completes in <10s.
- Storage backend (per D-01), serialization (per D-02), hashing (per D-03), location (per D-04), streaming (per D-06) all match decisions.
- No other crate depends on `akmon-journal` yet.
- `akmon-core` is **unchanged** — confirmed via diff against pre-Phase-1 state.
Notes for Cursor:
- Surface findings about existing audit event types in `akmon-core` before writing `Event` enum. Align vocabulary; do not yet retire duplicates (that's Item 6.10).
- Pace the work: types first, then store, then graph, then tests. Stop for approval between each.
- D-16 is binding: do not refactor `akmon-core` during this item under any circumstances.
---
 
**Item 1.3 — AGEF v0.1 spec alignment review**
 
Goal: Verify `akmon-journal`'s on-disk and in-bundle formats conform to the AGEF v0.1 spec. Resolve discrepancies (preferring spec updates at v0.1).
 
When to start: after Item 1.2 completes.
 
When done: SPEC.md and akmon-journal agree; spec version bumped if changes occurred.
 
---
 
### §6.3 Phase 2 — Capture wrappers
 
**Item 2.1 — Journaling provider wrapper**
 
Goal: `JournalingProvider<P: LlmProvider>` in `akmon-models::journaling` wraps any provider and captures requests/responses/streams into the journal. Per D-17, captures full attempt history via `AttemptRecord`.
 
When to start: after Phase 1 complete.
 
When done: all tests pass. Existing provider behavior preserved; only journal writes added. Retry history correctly captured for at least one backend (verified with mock that simulates rate limiting).
 
Notes for Cursor:
- Surface findings about the current `LlmProvider` trait first.
- D-17 sub-decision (2a/2b/2c on where retry capture lives) is settled during this item's design conversation, with code visible.
- D-06 (streaming capture) is settled.
---
 
**Item 2.2 — Journaling tool wrapper**
 
Goal: `JournalingTool<T: Tool>` in `akmon-tools::journaling` wraps tool I/O. Per audit finding D, this captures I/O only; permission events are emitted separately at session level in Item 3.1.
 
When to start: after Phase 1 complete.
 
When done: all tests pass. Existing tool behavior preserved.
 
Notes for Cursor:
- Surface current `Tool` trait shape first.
- Add `side_effects_manifest` as a defaulted method; do not retrofit every existing tool.
- This wrapper does NOT capture permission decisions — those are externalized in `akmon-query::concrete_permissions(...)` and emitted as PermissionGate events at session level.
---
 
### §6.4 Phase 3 — Session integration
 
**Item 3.1 — AgentSession takes a JournalHandle**
 
Goal: Session construction requires an explicit JournalHandle; session emits events to the graph at turn boundaries, provider calls (via wrapper), tool calls (via wrapper), retrievals, and permission decisions.
 
When to start: after Phase 2 complete.
 
When done:
- Full test session produces the expected event sequence.
- Existing tests pass unchanged.
- `akmon chat` produces journal artifacts on exit and prints the session head hash.
Notes for Cursor:
- Highest-risk item. Start with thorough audit of agent loop. Report findings. Wait for approval.
- D-13 (logging vs. journal) is settled.
- Do not refactor the loop. Instrument it.
- **Item 3.1 design resolutions (post-3.1a audit):**
  - **Decision 1 — Session granularity:** One AGEF session per `AgentSession`.
    - TUI multi-turn conversation: one session graph.
    - CLI single-turn invocation: one (smaller) session graph.
    - `SessionStart` is emitted at `AgentSession::new` construction time (before `run()`).
    - `SessionEnd` is emitted via `AgentSession::end()` when called, with `Drop` as safety-net emission when not explicitly ended.
    - The Drop-path `SessionEnd` has `summary_hash: None`. Callers that want a session summary in their `SessionEnd` event must call `end(summary_hash)` explicitly before drop.
  - **Decision 2 — RetrievalCall scope reduction (v2.0.0):**
    - For Item 3.1b, emit `ToolCall` for all dispatched tools, including retrieval-like tools (`semantic_search`, `search`, `read_file`, `web_fetch`, etc.).
    - Do **not** emit `RetrievalCall` in v2.0.0.
    - Retrieval classification and `RetrievalCall` emission are deferred to Item 3.3.
    - Verifiers (including Akmon's own `akmon verify` in Phase 4) MUST treat absence of `RetrievalCall` events as valid; presence of `ToolCall` events for tools that perform retrieval is the expected v2.0.0 shape.
  - **Decision 3 — SessionEnd centralization mechanics:**
    - Loop body remains instrumented, not refactored.
    - Minor lifecycle refactor is allowed in `AgentSession` only (construction/start, explicit end, drop-safety path).
    - If loop-body refactor is required, stop and escalate.
- **Revised instrumentation rule for Item 3.1b:**
  - Instrument the agent loop body.
  - Lifecycle concerns (`new`, `end`, `Drop`, journal-handle ownership) may be refactored minimally.
  - Any proposed `run()` loop-body refactor requires explicit approval.
---
 
**Item 3.2 — End-to-end session test**
 
Goal: Integration test covers a full session with all expected EventKind variants for v2.0.0 emitted (excluding `RetrievalCall`, deferred to Item 3.3).
 
When to start: alongside or immediately after Item 3.1.
 
When done: test passes, runs in <5s with mock provider and tool.
 
---

**Item 3.1c — Retrieval capture integration**

Status: **Deferred to Item 3.3 for v2.0.0.**

Reason: Retrieval classification is intentionally postponed to avoid concurrent `Tool` trait changes during active session-integration work. v2.0.0 emits `ToolCall` for retrieval-like tools and does not emit `RetrievalCall`.

---

**Item 3.3 — Add `is_retrieval` to `Tool` trait and emit `RetrievalCall` for matching tools**

Goal: Distinguish retrieval-class tool calls from action-class tool calls in the journal by adding `is_retrieval(&self) -> bool` to the `Tool` trait (default `false`) and emitting `RetrievalCall` vs `ToolCall` based on that flag.

When to start: After Item 3.1b lands and Item 3.2's end-to-end test passes. Before Phase 7 (release preparation). Item 3.3 absorbs the work originally scoped under Item 3.1c.

When done:
- `Tool` trait includes `is_retrieval` with default `false`.
- Retrieval tools opt in explicitly.
- Session integration emits `RetrievalCall` for retrieval-class tools and `ToolCall` for others.
- Existing behavior remains stable for tools that do not opt in.

Notes:
- This item is deferred from Item 3.1 to avoid two concurrent shape changes in `akmon-tools` during substrate/session integration.
- May require AGEF spec v0.1.2 clarification:
  "Implementations MAY emit ToolCall for tool invocations that the producer does not classify as retrieval. RetrievalCall is the preferred event when the implementation can identify retrieval semantics."

---
 
### §6.5 Phase 4 — Evidence operations
 
**Item 4.1 — `akmon verify`** (per D-12 output format)

**Scope (substrate-only for v2.0.0 Item 4.1):**

- **Invocation:** `akmon verify <session-id> [--journal <path>]` where `<session-id>` is the UUID assigned at `AgentSession` construction. `--journal` is optional and defaults to the per-user journal location (D-04).
- **Out of scope for 4.1:** Verifying an AGEF `.tar.zst` bundle file. Bundle verification is Item 4.3 (import path) or a narrowly scoped follow-up if import does not expose it cleanly.
- **Substrate checks:** Delegates to `akmon-journal` graph verification: parent chain, sequence, stored vs recomputed event hashes, stored head vs terminal event, referenced-object presence, **byte-level re-hash of object bytes** (per AGEF Section 13 step 5), and **SessionEnd** invariants (exactly one `SessionEnd`, last in sequence order).
- **Output:** Human-readable by default; explicit `--format json` (D-12) using Akmon-stabilized **VerifyReportV1** (not AGEF-normative). Exit codes: `0` success, `1` any verification violation, `2` usage error, `3` I/O or environment error — documented under `docs/src/reference/` (Item 4.1 command page).

**Item 4.1 — Design decisions (E1–E7) for traceability**

1. **E1 — Primary operand:** `akmon verify <session-id>` where `<session-id>` is the session UUID from `AgentSession` construction.
2. **E2 — Journal path:** Optional `--journal <path>`; when omitted, default is the per-user journal (D-04). No head-to-session index; head-oriented verification for shipped artifacts returns via Item 4.3 (bundle manifest embeds `session.head` and session id).
3. **E3 — Substrate vs bundle:** Item 4.1 verifies live on-disk journals only; bundle verify remains Item 4.3 (or follow-up).
4. **E4 — Object re-hash:** Extend `SessionGraph::verify` (Redb + in-memory symmetry) to read object bytes, re-digest, and record mismatches in `VerificationReport` (pre-step before CLI layers).
5. **E5 — SessionEnd invariants:** Extend verification walk to count `SessionEnd` events and assert a single terminal `SessionEnd` (findings in report; surfaced in CLI/JSON).
6. **E6 — JSON report:** **VerifyReportV1** in `akmon-cli` initially (shared crate only if multiple commands need it). Category strings are stable Akmon contract; schema documented under `docs/src/reference/` (Item 4.1 command page).
7. **E7 — Exit codes:** `0` success, `1` any verification violation, `2` usage error, `3` I/O or environment error — documented under `docs/src/reference/` (Item 4.1 command page).

**Item 4.2 — `akmon inspect`** (per D-12 output format)

**Scope (substrate-only for v2.0.0 Item 4.2):**

- **Invocation:** `akmon inspect <session-id> [--journal <path>] [--format <human|json>] [--resolve] [--verbose] [--binary <meta|hex|base64>]`.
- **Operand:** `<session-id>` is the UUID assigned at `AgentSession` construction (same addressing model as Item 4.1).
- **Journal path:** `--journal` optional; default is per-user journal location (D-04).
- **Substrate-only:** Item 4.2 reads on-disk journal sessions only. Bundle inspection remains Item 4.3 territory.
- **Output intent:** Human-readable event timeline by default; explicit JSON for CI/tooling; optional object-content resolution for hash fields.

**Item 4.2 — Design decisions (Q1–Q5) for traceability**

1. **Q1 — P0 wording alignment:** Inspect uses session UUID (not head hash). P0-5 wording updated accordingly.
2. **Q2 — Human output verbosity:** Default output is scannable summaries per event; `--verbose` expands event metadata and kind-specific detail.
3. **Q3 — Resolve behavior:** `--resolve` attempts to resolve **all** hash fields uniformly, with content-aware rendering (UTF-8 text preview vs binary-safe representation).
4. **Q4 — Binary display mode:** `--binary <meta|hex|base64>` controls non-UTF-8 rendering when `--resolve` is active; default `meta`. `--binary` without `--resolve` is usage error (exit code 2).
5. **Q5 — Filtering deferred:** Kind/range/limit filtering is intentionally out of Item 4.2 scope for v2.0.0.

**Item 4.2.1 — `akmon inspect` filtering flags** (deferred follow-up)

Goal: add `--kind <KIND>`, `--range <START..END>`, and `--limit <N>` filtering controls for inspect output.

When to start: After Item 4.4 (`akmon redact`) lands, or earlier if real-world inspect usage shows clear need.

Notes:
- Out of scope for v2.0.0 Item 4.2 initial ship.
- Item 4.2 ships full-session inspection first; filtering is additive follow-up.
 
**Item 4.3 — `akmon bundle export` and `akmon bundle import`** (per D-11 bundle format, AGEF spec)

**Item 4.3 — Design decisions (F1–F12) for traceability**

1. **F1 — Item structure and sequencing:** Item 4.3 remains one backlog item containing both bundle commands. Implementation proceeds sequentially within Item 4.3 (export-focused layers first, then import-focused layers), not as split 4.3a/4.3b tracks.
2. **F2 — Primary export operand:** `akmon bundle export` uses `<session-id>` (UUID), aligning Item 4.3 addressing with Items 4.1 and 4.2.
3. **F3 — Import behavior:** `akmon bundle import` mutates by default (ingests into local journal) and supports `--verify-only` for non-mutating verification.
4. **F4 — Bundle verification entrypoint:** Bundle verification for Item 4.3 is provided via `akmon bundle import --verify-only`. `akmon verify` remains substrate-only (`<session-id>` against on-disk journal).
5. **F5 — Manifest serialization + bundle layout:** `manifest.json` is JSON metadata per AGEF v0.1.1 §6. Item 4.3 implements AGEF v0.1.1 normative layout (`manifest.json`, `events.bin`, `objects/<hex>`), with `events.bin` using 4-byte big-endian length-delimited canonical-CBOR event framing.
6. **F6 — Round-trip strictness (v2.0.0):** Required guarantee is semantic equivalence (`event_count`, `object_count`, linkage, object hashes, and `session.head` invariants). Byte-identical tar.zst output is explicitly not required for v2.0.0.
7. **F7 — Session collision policy:** Default behavior is reject-on-collision when imported `session.id` already exists in target journal (verification violation exit path). Explicit remap is supported via `--rename-to <new-uuid>`.
8. **F8 — Object collision policy:** Content-addressed dedup with byte verification: if object hash already exists, importer reads existing bytes and verifies digest equality. Match => skip write. Mismatch => hard error (indicates local store corruption).
9. **F9 — Unknown extra files in bundle:** Strict by default. Unknown top-level or internal files are rejected unless `--allow-extra-files` is explicitly set.
10. **F10 — Compression determinism (v2.0.0):** No byte-level compression determinism target. Use zstd level 19 default and tar crate defaults; document that archive bytes may vary while semantic content remains stable.
11. **F11 — Bundle verification JSON schema:** Item 4.3 defines **BundleVerifyReportV1** (separate from `VerifyReportV1`) to represent bundle-specific validation categories (manifest/framing/unknown variant rules) without overloading substrate verify schema.
12. **F12 — File extension convention:** `akmon bundle export` defaults to `.akmon` output (tar.zst internally). `akmon bundle import` accepts `.akmon` and `.tar.zst` paths (and does not require a specific extension).

**Boundary note (Item 6.10):**
- The existing `akmon import` / `akmon export` commands (AKMON.md context sync) are not changed by Item 4.3. Any retirement, migration, or rename of those legacy context-sync commands is handled under Item 6.10 (`akmon-core` legacy retirement) with the broader v1.x command-surface cleanup.
 
**Item 4.4 — `akmon redact`** (per D-07)

**Scope (v2.0.0, journal-session input only):**

- **Architecture:** Derivative bundle only (R3). `akmon redact` reads one source session from the local journal and writes a new sanitized `.akmon` bundle. Source journal remains bit-identical.
- **Granularity:** Object-level redaction only for v2.0.0. Field-level and span-level redaction are deferred.
- **Selectors:** One or more explicit object hashes via repeatable `--object <hash>`. No pattern selectors or policy-profile selectors in v2.0.0.
- **Audit reason:** `--reason <text>` is required. Redaction without explicit operator reason is out of scope.
- **Sentinel substitution model:** For each selected object hash in the source session closure, the derivative bundle replaces references to that object with a canonical redaction sentinel object; event hashes and parent linkage are recomputed in the derivative artifact as normal AGEF content addressing requires.
- **Verification semantics:** Unchanged. `akmon verify <session-id>` continues to verify source journals; `akmon bundle import --verify-only` validates redacted bundles using existing AGEF integrity checks.
- **Bundle boundary:** Redacted bundles are ordinary AGEF bundles containing sentinel objects; no special import/export codepath or protocol mode.
- **Input operand boundary:** v2.0.0 `akmon redact` accepts session-id only. Direct bundle-to-bundle redaction is deferred to Item 4.4.1.

**Invocation (v2.0.0):**

```bash
akmon redact <session-id> \
  --output <path> \
  --object <hash> [--object <hash> ...] \
  --reason <text> \
  [--journal <path>] \
  [--format <human|json>]
```

- `<session-id>` required (UUID, same convention as Items 4.1–4.3).
- `--output` required (explicit destination path).
- `--object` required and repeatable (at least one).
- `--reason` required.
- `--journal` optional; default per D-04.
- `--format` default `human`.

**Exit codes (v2.0.0):**

- `0` derivative bundle written successfully
- `1` reserved (not currently emitted)
- `2` usage error (including missing required flags, output path exists, or selected `--object` hash not referenced in source session closure)
- `3` I/O or environment error (journal/session not found, read/write failures)

**Item 4.4 — Design decisions (R1–R10) for traceability**

1. **R1 — Architecture:** R3. Derivative bundle workflow only; no in-place journal mutation.
2. **R2 — Granularity:** Object-level redaction for v2.0.0. Field/span-level redaction deferred.
3. **R3 — Selector model:** Repeatable explicit `--object <hash>` flags; no pattern/policy selectors.
4. **R4 — Reason requirement:** `--reason <text>` is mandatory.
5. **R5 — Sentinel format:** Canonical-CBOR object payload:

   ```
   {
     "akmon_redacted": true,
     "original_hash": "<hex>",
     "original_size": <bytes>,
     "reason": "<text>",
     "redacted_at": "<rfc3339>"
   }
   ```

   Sentinel object hash is computed by the active hash algorithm (`sha256` or `blake3`) for the producing journal/bundle pipeline.
   Sentinel object format is Akmon-specific: sentinels are valid AGEF objects (canonical CBOR, content-addressed), but the `akmon_redacted` marker convention is not part of AGEF v0.1.1. Future AGEF versions may standardize redaction sentinels; until then, other AGEF readers may not interpret this marker.
6. **R6 — Reversibility:** One-way at the derivative bundle layer. Originals remain only in source journal; if that source is destroyed, redacted payload is unrecoverable by design.
7. **R7 — Verify behavior:** Existing verify flows unchanged and strict.
8. **R8 — Inspect behavior:** `inspect --resolve` detects sentinel content and renders redaction-aware fields/output without changing inspect's overall schema shape.
9. **R9 — Bundle handling:** No special bundle protocol mode; redacted bundles remain normal AGEF bundles.
10. **R10 — Input scope:** Bundle-path input for redact deferred.

**Item 4.4.1 — `akmon redact` on existing bundles** (deferred follow-up)

Goal: allow `akmon redact` to accept an existing bundle as input and produce a further redacted derivative bundle.

When to start: after Item 4.4 ships for v2.0.0 session-id/journal input.

Notes:
- Out of scope for v2.0.0 Item 4.4.
- Primary use case: forwarding partially redacted bundles with additional redactions.
- Must preserve AGEF verification semantics equivalent to Item 4.4 outputs.
 
Each item: design first, implement, document under `docs/src/commands/` or `docs/src/reference/` (per item; Item 4.1 command docs live in `docs/src/reference/`), verification gate of fmt+clippy+test.
 
---
 
### §6.6 Phase 5 — Replay
 
**Item 5.1 — PlaybackProvider and PlaybackTool** (inert, no real side effects)
 
**Item 5.2 — ReplayEngine** (per D-09: best-effort with divergence report)
 
**Item 5.3 — `akmon replay`**
 
---
 
### §6.7 Phase 6 — Diff
 
**Item 6.1 — SessionDiff algorithm** (per D-10: greedy same-kind-next)
 
**Item 6.2 — Diff rendering** (text, JSON, HTML — HTML self-contained)
 
**Item 6.3 — `akmon diff`**
 
---
 
### §6.8 Phase 6.5 — akmon-core cleanup
 
**Item 6.10 — Retire legacy audit/replay/evidence code in akmon-core**
 
*Added in v1.1 per Decision D-16.*
 
Goal: Remove the legacy hash-chained audit, replay metadata, and evidence validation code from `akmon-core`. Replace with thin renderers that produce the existing JSONL format from `akmon-journal` data, preserving compatibility for any external tools that consume the legacy format.
 
When to start: After Phase 6 (diff) is complete and before Phase 7 (release preparation) begins.
 
When done:
- The legacy `AuditChainRecord` JSONL output is produced by a renderer over `akmon-journal`, not by direct emission from the agent loop.
- Diffing the JSONL output of a v2.0 session against a v1.x session of equivalent shape shows compatible structure (forwards-compatible only; new fields permitted).
- `akmon-core` no longer contains audit/replay/evidence-specific code; only policy, FSM, sandbox, and shared primitives remain.
- All tests still pass.
Notes for Cursor:
- This is the deletion pass promised by D-16. Treat it as serious cleanup, not a refactor sprint. Surface findings before deletion. Identify each piece of legacy code, its consumer (if any), and confirm the new renderer covers the consumer's needs before removing.
- The renderer is a thin function: take a `SessionGraph`, walk events, produce JSONL records. It does not reproduce the *internal* hash chain; it produces the *output format* that external tools expected.
- If any external tooling (CI scripts, dashboards) consumes the legacy JSONL, identify it before deletion and verify the renderer satisfies its contract.
Constraint: this item is required before tagging v2.0.0. It is not optional cleanup; it closes the architectural debt D-16 deferred from Phase 1.
 
Commit: `refactor(core): retire legacy audit/replay/evidence; render from journal (Item 6.10)`.
 
---
 
### §6.9 Phase 7 — v2.0 release preparation
 
**Item 7.1 — README and positioning rewrite** (lead with §1–§2 thesis; three demos; quoted case study)
 
**Item 7.2 — docs expansion** (reproducibility, regulated-workflows, threat-model, command pages)
 
**Item 7.3 — CHANGELOG and release notes**
 
**Item 7.4 — Case study publication** (with tester written approval)
 
**Item 7.5 — DCO setup** (per D-14)
 
---
 
### §6.10 Phase 8 (optional) — Bisect and TUI
 
**Item 8.1 — `akmon bisect`** (post-v2.0)
 
**Item 8.2 — TUI views** (post-v2.0)
 
---
 
## §7 Ongoing discipline
 
- **Every PR answers the 8-question checklist** (traces to replay/diff/bisect/verify, no unapproved deps, no silent scope creep, existing behavior preserved, tests pass, test time under budget, no module > 800 lines, at least one deletion or deletion-candidate noted).
- **Every PR that changes user-visible behavior updates the docs in the same PR.** No doc debt.
- **Every PR that changes the wire format or storage format bumps AGEF spec.** Breaking changes bump major; additive bump minor.
- **Every PR title and commit message references the specific backlog item from §6** (e.g., `Item 1.2`, `Item 4.3`).
- **The file `docs/planning/AKMON_V2_DECISION_DOCUMENT.md` is the authoritative plan.** Modifications require a dedicated PR whose sole purpose is plan revision. No code PR may modify the decision document as a side effect.
- **Once a tester is quoted, they get a PR preview and a release-candidate build.** Case-study testers are insiders.
---
 
# Appendix A — AGEF v0.1 spec seed
 
**This is a seed, not a full spec.** Expand as Item 1.1 is executed. Publish under `radotsvetkov/agef/SPEC.md`.
 
## A.1 Purpose
 
AGEF (Agent Governance Evidence Format) defines a portable, content-addressed, tamper-evident record of an AI agent session. Designed to be produced by any agent tool and consumed by any evidence-handling system — SIEMs, compliance dashboards, review workflows, replay engines.
 
AGEF exists because AI agent sessions are increasingly consequential and existing log formats are inadequate: not tamper-evident, not reproducible, not portable.
 
## A.2 Design goals
 
- **Tamper-evident.** Any mutation to a recorded session is detectable by anyone with the session head hash.
- **Content-addressed.** Every artifact inside a session is referenced by its content hash.
- **Portable.** A session moves between machines as a single file with no external dependencies.
- **Verifiable offline.** Integrity checked without network access.
- **Tool-neutral.** Any spec-compliant agent produces AGEF; any spec-compliant tool reads AGEF.
- **Extensible.** Format carries a version; readers reject unknown versions unambiguously.
## A.3 Non-goals
 
- Not an agent runtime specification.
- Not a signature/cryptographic identity standard. Signing is out-of-scope for v0.1; expected in v0.2 via plugin standards like cosign.
- Not a model behavior certification.
## A.4 Format structure
 
An AGEF bundle is a `tar.zst` archive containing:
 
```
manifest.json             — bundle metadata, see §A.5
events.bin                — ordered sequence of Event records, see §A.6
objects/<hex>             — one file per content-addressed blob
```
 
## A.5 manifest.json
 
```json
{
  "agef_version": "0.1",
  "producer": {
    "name": "akmon",
    "version": "2.0.0"
  },
  "session": {
    "id": "<uuid v4>",
    "head": "<hash>",
    "created_at": "<rfc3339>",
    "ended_at": "<rfc3339>"
  },
  "hash_algorithm": "sha256",
  "object_count": <integer>,
  "event_count": <integer>
}
```
 
## A.6 Event structure
 
```
Event {
  parents: [Hash],       // hashes of predecessor events
  kind: EventKind,       // see §A.7
  emitted_at: timestamp,
  sequence: integer      // monotonic per-session, starts at 0
}
```
 
The event's own hash is computed over canonical CBOR encoding.
 
## A.7 EventKind variants (v0.1)
 
- `SessionStart { cwd_hash, config_hash }`
- `UserTurn { prompt_hash }`
- `ProviderCall { provider_id, attempts: [AttemptRecord], stream_hash? }` *(per D-17, attempts is the array of all retry attempts; the final successful one's response_hash is the logical "response" of the call)*
- `ToolCall { tool_id, input_hash, output_hash, side_effects_hash? }`
- `RetrievalCall { index_id, query_hash, results_hash }`
- `PermissionGate { policy_id, decision, context_hash }`
- `AssistantTurn { message_hash, tool_calls_hash? }`
- `SessionEnd { summary_hash? }`
`AttemptRecord` schema is defined in D-17.
 
Readers MUST reject bundles containing EventKind variants they do not recognize unless the major version matches and the kind is declared additive in the spec version.
 
## A.8 Hash algorithm
 
v0.1 REQUIRES SHA-256 by default. The manifest's `hash_algorithm` field is `"sha256"`. v0.1 readers MAY support `"blake3"` if the manifest declares it. Future versions MAY allow alternative algorithms.
 
## A.9 Serialization
 
- Events serialized as CBOR (RFC 8949). Canonical encoding per RFC 8949 §4.2.1.
- Object blobs stored as opaque bytes.
- manifest.json is UTF-8 JSON with LF line endings, sorted keys, no trailing whitespace.
## A.10 Verification procedure
 
A verifier given a bundle:
 
1. Extracts the archive.
2. Parses manifest.json. Rejects on version mismatch.
3. Reads events.bin. For each event:
   a. Computes event's canonical hash.
   b. Verifies all `parents` entries resolve to previously-seen event hashes.
   c. Verifies all content hashes inside `kind` resolve to files in `objects/<hex>`.
   d. For each referenced object, reads file and verifies hash matches filename.
4. Reports any failures.
A bundle passes verification when all events' computed hashes match their claimed linkages, all objects exist, all object hashes match their filenames, and the session's head event is reachable from the first SessionStart event.
 
## A.11 Versioning
 
- v0.x — pre-stable; breaking changes allowed.
- v1.0 — first stable major. Bundles produced against v1.0 readable by all v1.x readers.
- v2.0 — next breaking major. v1.x bundles MAY be readable by v2.x tools with explicit opt-in.
## A.12 Governance
 
Currently held by Rado Tsvetkov as benevolent dictator (per D-08). Intent to transition to core-maintainer model by v1.0. See GOVERNANCE.md.
 
## A.13 License
 
SPEC.md and all normative AGEF documentation are licensed CC BY 4.0. Reference implementations licensed per producing project.
 
---
 
# Appendix B — Glossary
 
- **Akmon** — the AI coding agent tool; this document's subject.
- **Session** — one logical agent run: from `SessionStart` to `SessionEnd`.
- **Journal** — Akmon's local storage of content-addressed objects and session graphs.
- **Object** — an immutable content-addressed blob in the journal.
- **Event** — one entry in the session graph, referencing objects and parent events by hash.
- **Head** — the hash of the most recent event in a session; canonical identifier of a complete session.
- **Bundle** — a portable serialization of a session: manifest + events + objects in a `tar.zst` archive.
- **AGEF** — Agent Governance Evidence Format; the public spec for bundles.
- **AttemptRecord** — captured detail of a single provider HTTP attempt within a logical ProviderCall (per D-17).
- **Verify** — prove a session's internal integrity using only its bundle.
- **Inspect** — walk a session's event list for human or machine reading.
- **Export** — produce a bundle from the journal.
- **Import** — load a bundle into a journal.
- **Redact** — produce a derivative bundle with sensitive content scrubbed.
- **Replay** — re-run a session in one of three modes (strict / regenerate / dry).
- **Diff** — compare two sessions and classify their pairwise event differences.
- **Bisect** — binary-search a sequence of sessions to find the first where behavior changed.
---
 
# Appendix C — What success looks like
 
Three months after v2.0 ships:
 
- At least one quoted case study published.
- At least 500 GitHub stars (from baseline today).
- At least 3 external contributors (non-Rado commits merged).
- At least one non-Akmon project emits AGEF bundles or reads them.
- Akmon mentioned in at least one mainstream regulated-engineering discussion.
- 5 specific organizations using Akmon (not "downloaded" — using).
If roughly met: positioning is working.
If none met: positioning failed; revise or pivot.
If 1–2 met: mixed signal; iterate on positioning, not code.
 
---
 
**End of document.**
