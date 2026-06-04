# Akmon — Strategic Improvement Plan & Deep Analysis

**Status:** PROPOSAL / ADVISORY. This document is **not authoritative** and does **not**
modify `AKMON_V2_DECISION_DOCUMENT.md`. The decision document remains the single source of
truth; any change to LOCKED positioning or to substrate invariants requires a dedicated
revision PR per `.cursor/rules/04-v2-scope-discipline.mdc` and `02-substrate-invariants.mdc`.

**Purpose:** A critical, multi-role analysis of the project as it actually exists today
(v2.1.0), an assessment against current enterprise needs and market/standards trends, and a
prioritized improvement plan to make Akmon both community-praised and enterprise-adopted —
without breaking the locked thesis.

**Method:** Full read of the decision document, README, architecture docs, and a code-level
audit of all 12 crates (substrate, agent loop, providers, tools, security, CLI, CI/release,
tests, docs), cross-referenced against 2026 enterprise AI-governance trends (EU AI Act
Article 12/19/26, NIST AI RMF, SOC 2), the competitive landscape (Cursor, Copilot, Claude
Code, Microsoft `agent-governance-toolkit`), and the emerging standards picture
(OpenTelemetry GenAI semantic conventions, the open "AI Forensics Specification").

**Document version:** 0.1 — June 2026.

---

## §A. Verdict

Akmon is a genuinely differentiated, well-engineered substrate with a strong thesis and a
credibility problem at exactly the layer the thesis depends on. Two market shifts matter:

1. **Enterprises now buy "defensible evidence," and Akmon produces "logs," not "evidence."**
   Tamper-evidence is self-produced by the same binary that performed the work. There is no
   cryptographic signing, no external anchoring, and no independent attestation (signing was
   deferred in D-05). An auditor's first objection — *"logs from the party being audited need
   independent verification"* — currently has no answer. This is the highest-leverage gap.

2. **A bespoke format (AGEF) is swimming against a converging current.** OpenTelemetry GenAI
   semantic conventions are becoming the de-facto capture schema; Microsoft's
   `agent-governance-toolkit` and an open "AI Forensics Specification" (DSSE-signed
   attestations profiling OTEL/OCSF/SPIFFE/NIST) occupy the same conceptual space while
   reusing standards instead of inventing one.

The work is **not** a pivot. The thesis is *more* timely than when written (EU AI Act
high-risk obligations activate **August 2, 2026**). The work is: close the trust gaps that
make a security team say "yes," and interoperate instead of isolate.

---

## §B. What is genuinely strong (baseline to protect)

- **Substrate verification is real.** Object byte re-hash (AGEF §13 step 5), parent chain,
  sequence, head consistency, and SessionEnd invariants — `crates/akmon-journal/src/session_graph.rs:92-183`.
- **Security posture above the field.** Canonical-before-prefix path confinement with symlink
  rejection (`crates/akmon-core/src/sandbox.rs:92-119`); fail-closed policy, deny-by-default,
  explicit-deny-wins (`crates/akmon-core/src/policy.rs:274-277,461-465`); `Secret<T>` with no
  `Debug` + zeroize (`crates/akmon-core/src/secret.rs`); argv-only shell with metachar
  rejection, allowlist, 30s/512KiB caps (`crates/akmon-tools/src/shell.rs`); SSRF guards in
  `web_fetch`.
- **Tool args schema-validated before dispatch** (`crates/akmon-tools/src/schema_validate.rs`,
  invoked at `crates/akmon-query/src/session.rs:1645`).
- **Test depth:** ~1,051 test functions, mock-provider E2E, strong journal/bundle/replay/diff/CLI coverage.
- **Provider breadth with explicit HTTP timeouts** (`crates/akmon-models/src/http_client.rs:9-12`),
  full attempt-history capture (D-17 `AttemptRecord`).
- **High documentation and decision discipline.**

---

## §C. Critical findings (evidence-backed)

### §C.1 Trust / credibility gaps (undermine the core thesis)

1. **No independent verifiability ("logs vs. evidence").** Tamper-evidence proves internal
   consistency only; no signing, no detached attestation over the head hash, no
   transparency-log/timestamp anchoring.
2. **Bundle verification is weaker than journal verification.** `read_bundle` does not re-hash
   objects, re-walk the chain, or check manifest head/counts; `BundleError::ObjectHashMismatch`
   and `HeadMismatch` are defined but never used (`crates/akmon-bundle/src/error.rs:56,60`).
   The bundle is the artifact that leaves the machine — verified less strictly than the local copy.
3. **No operator-identity binding.** EU AI Act Art. 12/19 require resolving actions to a human
   identity, model version, and policy version. Akmon captures a session UUID/config but no
   attested operator identity or first-class policy-version evidence field.
4. **No retention/lifecycle model.** Art. 26(6) mandates ≥6 months; finance (DORA/MiFID II) and
   health (MDR) push 5–10 years. The journal has no retention policy, archival/WORM target, or
   "lifetime across redeploys" guarantee.

### §C.2 Plan-vs-reality divergences (require escalation — see §J)

5. **Item 6.10 not done, yet v2.0.0 and v2.1.0 are tagged.** Decision doc §4 / §6.8 make Item
   6.10 (retire legacy audit/replay/evidence; render from journal) a hard gate "required before
   tagging v2.0.0." In reality legacy `akmon-core` audit/replay/evidence/SLO code still exists
   and is still emitted from the loop in parallel with the journal
   (`crates/akmon-query/src/session.rs:2922-2954`; CLI still calls
   `write_audit_jsonl(session.audit_events())`). No `render_audit_from_journal` exists. Two
   audit systems coexist with drift risk.
6. **Required CI inference matrix absent.** §4 point 5 makes an Ollama run + an API-inference run
   a v2.0 shippable condition; no workflow contains either (`.github/workflows/*`).
7. **`spawn_subagent` contradicts a LOCKED non-goal.** §1.2 / §3.4 (P3-4) exclude multi-agent
   orchestration ("one agent, one session, one artifact"), but
   `crates/akmon-query/src/subagent_tool.rs` spawns nested sessions with their own journal handles.

### §C.3 Engineering hygiene (violates `99-non-negotiable` / decision doc §7)

8. **Monolith modules far over the 800-line rule.** `crates/akmon-cli/src/main.rs` ≈ **8,401**
   lines; `crates/akmon-query/src/session.rs` ≈ **7,030**; six `akmon-models` files >800;
   `crates/akmon-tui/src/runner.rs` ≈ 1,998. Primary barrier to the "≥3 external contributors" goal.
9. **Production `panic!`/`expect` outside startup-validation.** HTTP client construction panics
   (`crates/akmon-models/src/anthropic.rs:104-105` pattern, also Ollama/OpenAI/Bedrock);
   `crates/akmon-replay/src/engine.rs:698` `expect("projection encode")`; repeated
   `out.last_mut().expect("pushed")` in `crates/akmon-diff/src/comparison.rs`.
10. **AGEF version-string drift** (`0.1.1` in journal vs `0.1` in `JournalMeta`/replay/diff reports).
11. **Performance/scale ceilings:** O(n) append via `history().len()` and full-table scans
    (`crates/akmon-journal/src/session_graph.rs:402-403,485-510`); unbounded object loads (no
    streaming `ObjectStore` API; bundle fully materialized in RAM); single-process redb lock
    (`crates/akmon-journal/src/object_store.rs:147-148`).

### §C.4 Enterprise-readiness & supply-chain gaps

12. **Supply chain thin for a "trust" product:** no release checksums, no signing/SLSA
    provenance, no SBOM, no `cargo audit`/`cargo deny`, no Windows build, no crates.io/Homebrew.
13. **MCP client has no auth** (HTTP JSON-RPC only, no `Authorization` header, no stdio/SSE;
    `crates/akmon-tools/src/mcp_client.rs:109-116`). The 2026 enterprise pattern is OAuth/SSO MCP gateways.
14. **Compliance is narrative, not mapped.** Docs name DO-178C/IEC 62304/ISO 26262/SOC 2/CMMC
    without a control-to-feature crosswalk — the exact artifact the Platform/SRE persona needs.

---

## §D. Multi-role critique (questions self-answered)

- **CISO:** "If the agent writes its own audit log, why trust it over its console output?" —
  Today, you shouldn't beyond internal consistency. Fix = detached signatures + anchoring +
  an independent verifier.
- **Compliance auditor (EU AI Act):** "Show operator identity, model version, policy-version
  hash, and prove the record predates the action." — Identity and synchronous write-before-act
  are not first-class yet.
- **Staff Rust engineer:** "Can I land a feature without touching a 7,000-line file?" — No.
- **Platform/SRE (adoption multiplier):** "Signed, scanned, SBOM, behind our MCP gateway, with
  retention, plus a control mapping?" — None yet; these are the literal blockers to internal advocacy.
- **OSS maintainer:** "Is the format a standard I can build on?" — Published but isolated; needs
  OTEL interop + conformance tests + a non-Akmon producer/consumer.
- **Enterprise buyer:** "Why this over Microsoft's free toolkit?" — Akmon is the agent itself
  with evidence built in, local-first/air-gap-capable, regulated-specific — a real wedge *if* the
  trust gaps close.

---

## §E. Improvement plan

Four tracks, three horizons. All items framed to stay inside the LOCKED thesis and to be
**additive** to the substrate (signing as sidecar, OTEL as exporter, identity as new fields)
unless explicitly flagged for decision. Priorities: P0 first, P1 core, P2 differentiator,
P3 optional / needs-decision.

### Track 1 — Close the trust gap (the moat)

- **[P0] Native session signing (finish D-05).** Detached signature over the session head hash
  via Sigstore/cosign keyless *and* offline GPG/x509, written as a sidecar. Manifest gains a
  `signatures[]` field (additive; AGEF v0.1.2). Turns "logs" into "evidence."
- **[P0] Standalone bundle verification parity.** Wire `ObjectHashMismatch`/`HeadMismatch`; make
  `bundle import --verify-only` (and a `verify <bundle>` path) re-hash every object, re-walk the
  chain, and check manifest counts/head without importing. Closes §C.1.2.
- **[P1] Independent, dependency-free verifier.** A small separately-distributed `agef-verify`
  (static binary and/or ~200-line reference verifier in the AGEF repo; consider WASM for
  browser drag-and-drop). An auditor verifies with a tool that is **not** Akmon.
- **[P1] Transparency-log / timestamp anchoring (per session, optional).** Anchor head hashes to
  Rekor or an RFC-3161 TSA to prove existence-at-time and prevent backdating.
- **[P2] Operator-identity & policy-version evidence.** Attested operator identity (env/OIDC) and
  a `policy_version_hash` (additive event/field). Satisfies Art. 19 fields.

### Track 2 — Interoperate (saves AGEF from irrelevance)

- **[P1] OpenTelemetry GenAI export/import.** `export --format otel-genai` + an importer for OTEL
  GenAI JSONL. Makes Akmon the tamper-evident + replayable layer over the schema everyone emits.
- **[P1] Compliance crosswalk artifacts.** Machine-checkable mappings: EU AI Act Art. 12/19/26 ↔
  evidence fields; NIST AI RMF (MEASURE 2.7 / MANAGE 4) ↔ features; SOC 2 CC; per-domain notes
  for DO-178C / IEC 62304 / ISO 26262.
- **[P2] MCP enterprise hardening.** Auth headers (bearer/OAuth), stdio transport, gateway docs.
- **[P3 — needs decision] Position AGEF as a profile over OTEL GenAI / aligned with the AI
  Forensics Spec** (touches §1/§3 positioning → escalate).

### Track 3 — Engineering hygiene & supply chain

- **[P0] Execute Item 6.10** (retire legacy dual-write; render legacy JSONL from `akmon-journal`)
  — after the §J ruling.
- **[P0] Decompose monoliths** (`main.rs`, `session.rs`) into ≤800-line modules per §7.
- **[P0] Remove production panics; unify the AGEF version constant.**
- **[P0] Supply chain:** signed releases + checksums + SBOM + `cargo deny`/`cargo audit` in CI +
  OpenSSF Scorecard + Windows build.
- **[P1] Wire the required CI inference matrix** (Ollama + one API) — closes §C.2.6.
- **[P1] Scale hardening:** streaming/size-capped `ObjectStore` API (Item 6.V); fix O(n) append
  (store sequence in head row); stream bundle I/O.
- **[P2] Property/fuzz tests for the verifier** (proptest + cargo-fuzz on event/CBOR/bundle parsing).

### Track 4 — Adoption & community

- **[P1] Distribution:** crates.io, Homebrew tap, signed install script.
- **[P1] Reference verifier demo + 3 honest case studies** showing produce → sign → ship →
  independently verify → replay on a real regulated-style repo.
- **[P2] AGEF conformance test suite** in the spec repo (precondition for a non-Akmon AGEF user).
- **[P2] Governance dashboard / SIEM export** once OTEL export exists (likely community-buildable).

### Horizons

- **Now → v2.2 ("Trust & Hygiene"):** Track 1 P0s + Track 3 P0s.
- **Next → v2.3 ("Interoperate & Comply"):** Track 2 P1s, identity/retention, CI matrix, scale
  hardening, distribution.
- **Later → v3.0 ("Ecosystem"):** conformance suite, anchoring at scale, positioning-level
  decisions (subagents, AGEF-as-profile, any managed verification service — all need decision-doc revision).

---

## §F. Success metrics (extend Appendix C of the decision document)

Keep existing targets and add: (1) an independent party verifies an Akmon bundle with a
non-Akmon tool; (2) one published EU-AI-Act/SOC-2 control mapping referenced by a real
reviewer; (3) signed, SBOM-bearing releases with an OpenSSF Scorecard at/above a stated
threshold; (4) zero modules >800 lines; (5) CI proves a session replays identically on a clean machine.

---

## §G. Risks / explicit non-actions

- Do not chase Cursor/Copilot/Claude Code on general coding UX or model quality (forbidden by §1.2).
- Do not expand into multi-agent orchestration or hosted SaaS to "look modern" (LOCKED non-goals;
  the `spawn_subagent` code is already over this line — see §J).
- Do not break the substrate to interoperate; OTEL/signing/identity all fit as additive
  exporters/sidecars/fields. Keep the merkle core sacred.
- If forced to pick one thing: **native signing + an independent verifier** (Track 1 P0/P1).

---

## §H. Escalations (product-owner rulings required before building)

1. **Item 6.10 / dual-audit divergence:** (a) execute 6.10 now as overdue debt, (b) revise §4 to
   drop the gate, or (c) keep coexistence and document it?
2. **`spawn_subagent` vs. the "one agent, one session" non-goal:** thesis text stale, or code out
   of scope and slated for removal/gating?
3. **CI inference matrix (§4 pt 5):** confirm we add it.
4. **AGEF-as-profile-over-OTEL** and **signing scope (cosign/Sigstore vs GPG-only)**: positioning
   and substrate-version decisions.

This document intentionally does not modify `AKMON_V2_DECISION_DOCUMENT.md`.

---

**End of document.**
