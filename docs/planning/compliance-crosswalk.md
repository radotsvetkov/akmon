# Akmon Item 9.4 — Compliance Crosswalk: AGEF evidence capabilities → regulatory logging & record-keeping obligations

**Status:** DRAFT for internal review (not normative; pending legal review before any external use).
**Date:** 2026-06-05. **Reference:** AGEF v0.1.2 (`AGEF-SIG-v1`); `agef-verify`; `akmon otel import`.

## 1. Scope and disclaimer (read this first)

This document maps Akmon's actual technical capabilities (the AGEF tamper-evident session journal,
`.akmon` bundles, offline Ed25519 signatures, the standalone `agef-verify`, deterministic replay,
and OTEL GenAI import) to the **evidence / record-keeping / logging obligations** in three
frameworks: the **EU AI Act**, the **NIST AI Risk Management Framework** (incl. the Generative AI
Profile), and **SOC 2** (AICPA Trust Services Criteria).

Binding reading rules for every row below:

1. **This is a scoping aid, not a compliance certificate.** Akmon is **one technical control** — an
   evidence layer. "Maps to an obligation" means Akmon can *produce or strengthen the evidence* that
   the obligation is met; it does **not** mean an organization using Akmon *is* compliant.
2. **Compliance requires much more than logging** — risk management, human oversight, data
   governance, transparency, conformity assessment, organizational policy. None of that is in
   Akmon's scope; Akmon is silent on most of each framework.
3. **This is not legal advice.** Article numbers, retention floors, and criterion text are reproduced
   from official sources (§7) and may be amended; obligations depend on your role (provider vs.
   deployer), risk classification, and sector law. Consult qualified counsel.
4. **"Provides the evidence" ≠ "satisfies the obligation."** Where Akmon only partially helps —
   especially **retention** (Akmon produces durable signed artifacts, but the *org* must operate
   retention/WORM storage + a policy) and **operator-identity binding** (not yet first-class) — the
   gap column says so.
5. **Microsoft comparison is descriptive, not a teardown.** Microsoft's Agent Governance Toolkit
   (AGT) audit trail **is genuinely tamper-evident** (SHA-256 hash chain + Merkle tree, append-only).
   Azure Confidential Ledger **does** provide signed, independently-verifiable Merkle receipts. The
   comparison states precisely where Akmon differs (asymmetric offline signature, standalone no-cloud
   verifier, deterministic replay, agent-native capture) — not whether Microsoft's tools are "bad."

**Honest-capture model.** `akmon otel import` turns *any* agent's OpenTelemetry GenAI telemetry into
a signed AGEF bundle, but records a truthful `capture_level`: `full` vs. `structural`/metadata-only.
Verifiers surface this and can *require* full capture (`--require-capture full`). An imported bundle
can be cryptographically authentic yet **structurally** incomplete (prompts/responses redacted or
never emitted by the source). Several rows depend on this distinction; it is not glossed over.

## 2. Crosswalk A — EU AI Act (high-risk obligations; main application date **2 August 2026**, Art. 113)

| Obligation | What it requires | Akmon/AGEF capability (evidence) | Honest gap | Microsoft comparison |
|---|---|---|---|---|
| **Art. 12(1)** — automatic logging over the system lifetime | High-risk systems "shall technically allow for the automatic recording of events (logs) over the lifetime of the system." | AGEF auto-records every prompt/response/tool-call/file-change as a hashed entry in a merkle-linked chain; the head is a tamper-evident identifier. `akmon otel import` produces an equivalent signed journal from any agent's OTEL telemetry. | Akmon logs **agent sessions**, not the whole system's operational telemetry. "Lifetime of the system" is an org-level program of continuous capture + storage Akmon does not orchestrate. Imported bundles inherit the source's completeness (`capture_level`). | AGT auto-records into a SHA-256 hash chain + Merkle tree (tamper-evident, append-only); Foundry emits OTEL spans. Difference is the signing/verification model, not capture. |
| **Art. 12(2)** — logs support risk identification, post-market & operation monitoring | Logged events relevant to (a) risk/substantial-modification identification, (b) Art. 72 post-market monitoring, (c) Art. 26(5) operation monitoring. | Content-addressed chain = exact, ordered, verifiable record; deterministic replay (native sessions) reconstructs the run for analysis. | Akmon is the **evidence substrate**; it does not perform monitoring/anomaly detection/post-market analysis. | Comparable raw capture; neither AGT nor Foundry runs the org monitoring program. |
| **Art. 12(3)** — minimum log fields for biometric ID (incl. **human verifier identity**, Art. 14(5)) | Record use-period, reference DB, matched input, and the natural persons who verified results. | AGEF reliably captures timestamps, tool/data references, inputs/outputs → use-period, reference-DB, matched-input. | **Operator/human-verifier identity is NOT first-class in AGEF yet** — the chain proves *what happened*, not *which verified human* did it. Art. 12(3)(d)/14(5) is **not** satisfied by Akmon today. | AGT/Foundry likewise don't natively bind a verified human-operator identity; an app/IAM concern across all three. |
| **Annex IV §6** — technical docs incl. **"test logs… dated and signed by the responsible persons"** | Dated, signed test logs/reports in the technical documentation file. | A `.akmon` bundle of a test session is a self-contained, dated, **Ed25519-signed** artifact; `agef-verify` confirms integrity + signature offline with no Akmon/cloud install. | Akmon signs with the **key it is given**; the org runs key management and binds the key to a responsible person. Akmon produces one element of Annex IV; the other sections are out of scope. | AGT signs with **HMAC (symmetric/shared-secret)** + Merkle inclusion proofs — **no asymmetric detached signature** verifiable without the secret, **no standalone offline verifier**. Azure Confidential Ledger gives signed offline-verifiable receipts but is **cloud-locked** and **not agent-aware**. |
| **Art. 19(1)** — **providers** keep auto-generated logs ≥ **6 months** | Keep Art. 12 logs "for a period appropriate…, of at least six months," unless other law applies. | Akmon outputs durable, portable, offline-verifiable artifacts whose integrity is provable at any future date via the signed head; local-first/air-gap capable for sovereign retention. | **Akmon does not operate retention** — no WORM/immutable storage, lifecycle, legal hold, or 6-month enforcement. The org must store/retain bundles. Akmon makes retained logs *trustworthy*, not *retained*. | AGT/Foundry data lives in the customer's Azure sink (App Insights default 90-day); retention is the org's job there too. Akmon's edge is **verifiable, portable, off-cloud** retention artifacts. |
| **Art. 26(5)** — **deployers** monitor operation; report per Art. 72 | Monitor operation; report serious incidents upstream. | The signed session journal is the deployer's authentic record to support monitoring and substantiate an incident report with tamper-evident evidence. | Monitoring/reporting are **processes**, not artifacts; Akmon supplies evidence, not the function. | Same posture; AGT's tamper-evident trail similarly supports substantiation. |
| **Art. 26(6)** — **deployers** keep auto-generated logs ≥ **6 months** | Keep logs "for a period appropriate…, of at least six months," unless other (esp. data-protection) law applies. | Same as Art. 19(1): portable, signed, offline-verifiable bundles surviving long-term storage; air-gap friendly for data-residency constraints. | Same retention gap — org must operate retention/WORM. Data-protection law may extend/constrain. | Same as Art. 19(1). |

## 3. Crosswalk B — NIST AI RMF 1.0 (AI 100-1) + Generative AI Profile (AI 600-1)

Voluntary, outcome-based. Akmon's strongest fit is **MEASURE 2.8** (transparency/accountability —
the Playbook references "audit logs"; note this is 2.8, not 2.7 which is security/resilience).

| Subcategory | What it requires | Akmon/AGEF capability (evidence) | Honest gap | Microsoft comparison |
|---|---|---|---|---|
| **MEASURE 2.8** — transparency/accountability risks examined & documented (Playbook: maintain "histories, audit logs") | Instrument for measurement via histories/**audit logs**; enable accountability tracing across AI actors. | Core fit: immutable, content-addressed audit log of every step, with a verifiable head + offline `agef-verify`; deterministic replay examines what actually happened. | "Examined and documented" implies human analysis Akmon doesn't produce; cross-actor accountability needs operator-identity (not yet first-class). | AGT directly targets this (Merkle audit chain + "Decision BOM"); distinction is asymmetric-signed + standalone-verifiable + replayable vs. HMAC + inclusion-proof, no replay. |
| **MANAGE 4.1** — post-deployment monitoring incl. **change management** | Implement monitoring + change management with captured records. | AGEF captures every **file-change**/tool action → tamper-evident change-management evidence; bundles = durable per-session record. | Akmon is the **record**, not the monitoring plan / appeal-override / decommissioning process. | AGT governance events + Foundry observability address parts; Foundry has no documented replay/signed-head verification. |
| **MANAGE 4.3** — incident/error tracking "followed and documented" | Track/respond/recover from incidents, documented. | Signed AGEF bundle = high-integrity forensic evidence; native sessions **deterministically replay** to reproduce failures. | Akmon doesn't run the IR process. | AGT trail aids substantiation; **deterministic replay is Akmon-specific** for root-cause. |
| **GOVERN 1.4** — risk process established via transparent policies/controls | Document & govern the risk process transparently. | Signed verifiable artifacts evidence that documented procedures were actually exercised. | Akmon doesn't author policies/the governance program. | Comparable — all are evidence sources. |
| **GenAI Profile (AI 600-1)** — pervasive **content-provenance** emphasis | Track origin/history/authenticity of generative content. | Strong fit: content-addressed + merkle-linked → cryptographic provenance + integrity for generated content and producing steps, verifiable offline. | Establishes provenance of the **recorded session**; not a C2PA/watermarking system for downstream media; imported provenance is only as complete as `capture_level`. | AGT = tamper-evident provenance of actions; Foundry = spans. Akmon's content-addressed portable signed bundle is the closer fit to offline auditor-verifiable provenance. |

## 4. Crosswalk C — SOC 2 (AICPA 2017 Trust Services Criteria, rev. 2022)

Akmon is a **source of audit evidence** for CC7.x (monitoring/incident) and CC8.1 (change mgmt).

| Criterion | What it requires | Akmon/AGEF capability (evidence) | Honest gap | Microsoft comparison |
|---|---|---|---|---|
| **CC4.1** — ongoing/separate evaluations that controls function | Monitor that controls are present & functioning. | Verifiable AGEF bundles = durable, independently-checkable evidence; `agef-verify` confirms integrity **without trusting Akmon or any cloud**. | Akmon doesn't perform the evaluations; it supplies tamper-evident inputs. | Confidential Ledger gives independently-verifiable evidence too but cloud-bound; AGT evidence needs the shared HMAC secret to fully trust. |
| **CC7.1** — detect config changes / new vulnerabilities | Detect config changes/vulnerabilities. | AGEF's recorded file-changes/tool actions = tamper-evident trail of what an agent changed. | Akmon is **not** a vuln scanner / FIM / config monitor. | Out of scope for AGT/Foundry too. |
| **CC7.2** — monitor for & analyze anomalies | Monitor/analyze anomalies as security events. | A trustworthy unaltered record is the substrate anomaly analysis runs on. | Akmon does **not** detect/analyze anomalies (no SIEM/alerting). | AGT/Foundry don't replace a SIEM either. |
| **CC7.3** — evaluate events → incidents | Triage events into incidents. | Signed bundles + replay = high-integrity material to evaluate. | Akmon doesn't triage/decide dispositions. | Comparable; replay is Akmon-specific. |
| **CC7.4** — execute an incident-response program | Run IR (understand/contain/remediate/communicate). | Tamper-evident, replayable evidence supports "understand" + post-incident comms with auditable facts. | Akmon is not an IR program/orchestration. | Same posture. |
| **CC7.5** — recover from incidents (root cause; prevent recurrence) | Recover; root-cause; prevent recurrence. | **Deterministic replay** of native sessions = strong root-cause aid; signed records substantiate recovery changes. | Akmon doesn't perform recovery/preventive changes. | **Replay is a distinct Akmon advantage**; AGT/Foundry have no documented replay. |
| **CC8.1** — authorize/test/document/implement changes | Controlled, documented, traceable change management. | AGEF records agent change actions as a tamper-evident ordered chain = strong "documents/tests/implements" evidence; signed test-session bundles evidence "tests." | Akmon does **not** enforce authorization/approval/SoD or a change workflow; approval-to-actor traceability needs your CM system + identity. | AGT's Merkle chain = comparable change evidence; Akmon adds asymmetric signature + standalone verifier + replay. Neither replaces the approval workflow. |

## 5. What Akmon uniquely contributes for EU-AI-Act (and general audit) evidence

1. **Asymmetric, detached, offline signature over the session head (`AGEF-SIG-v1`, Ed25519)** — a
   third party verifies authenticity with **only the public key**, no shared secret. The concrete
   difference from a hash-chain/HMAC approach (AGT) where tamper-*evidence* exists but third-party
   verification needs the toolkit and/or the shared secret.
2. **A standalone verifier (`agef-verify`) with no Akmon and no cloud dependency** — evidence stays
   checkable for the 6-month+ retention window even offline/air-gapped or if the vendor disappears.
   The difference from cloud-locked verifiable ledgers: equivalent assurance, but portable + sovereign.
3. **Portable, self-contained `.akmon` bundles** — hand a regulator one offline-verifiable file, not
   access to a live cloud tenant.
4. **Deterministic replay of native sessions** — reproduce the run, not just read logs; materially
   stronger for root-cause (CC7.5, MANAGE 4.3) and substantiating "what the system did" (Art. 12(2)).
   No documented equivalent in Foundry telemetry or AGT.
5. **Agent-native + honest cross-vendor onboarding** — AGEF treats prompts/responses/tool-calls/
   file-changes as first-class; `akmon otel import` extends signed verifiable evidence to any
   OTEL-instrumented agent while **honestly labeling** `capture_level` and letting verifiers
   **require** full capture — avoiding the trap of a cryptographically valid bundle masquerading as
   content-complete.

In one line: **Akmon turns agent activity into evidence that is tamper-evident, third-party-verifiable
offline, portable, and replayable — the properties an EU AI Act auditor needs to trust a retained log
years later. It does not make you compliant.**

## 6. Honest gaps / not-yet (belongs in any auditor-facing use)

- **Retention and WORM are the organization's job.** Akmon produces durable signed artifacts; it does
  not provide immutable/WORM storage, lifecycle enforcement, the 6-month floor (Art. 19 / 26(6)),
  legal hold, or a retention policy. Pair Akmon with object-lock storage + a retention schedule.
- **Operator/human-identity binding is not first-class yet.** The chain proves *what happened and that
  it is unaltered*; it does not yet cryptographically bind a *verified human identity* to each step.
  Directly limits Art. 12(3)(d)/14(5) (biometric human-verifier) and weakens cross-actor accountability
  (MEASURE 2.8) and approval-to-actor traceability (CC8.1). Identity must come from your IAM/CM today.
- **Structural-capture caveat for OTEL imports.** Imported bundles are only as complete as the source
  telemetry. `capture_level: structural` = metadata/shape without full content; the bundle is authentic
  but not content-complete. For high-risk evidence, require `--require-capture full`.
- **Akmon ≠ the rest of compliance.** No risk-management system (Art. 9), data governance (Art. 10),
  human-oversight design (Art. 14), transparency-to-affected-persons, conformity assessment, or SOC 2
  access controls (CC6) / governance (CC1) / risk assessment (CC3). Akmon covers the logging/evidence slice.
- **Scope of "system."** AGEF logs agent sessions, not an entire system's operational telemetry.
- **Key management is on you.** `AGEF-SIG-v1` is only as strong as how the org generates/protects/
  rotates/attributes its signing keys. A signature only means "signed by whoever held this key."

## 7. Sources

**EU AI Act (Reg. (EU) 2024/1689; EUR-Lex consolidated text):** Art. 12 record-keeping
(artificialintelligenceact.eu/article/12/); Art. 19 auto-generated logs (/article/19/); Art. 26
deployer obligations incl. §6 ≥6-month (/article/26/); Annex IV technical documentation incl. §6
(/annex/4/); Art. 113 application dates — high-risk 2 Aug 2026 (/article/113/); official text
eur-lex.europa.eu/eli/reg/2024/1689/oj.

**NIST AI RMF:** AI 100-1 (nvlpubs.nist.gov/nistpubs/ai/nist.ai.100-1.pdf); AI RMF Core subcategory
outcomes (airc.nist.gov/airmf-resources/airmf/5-sec-core/); Playbook (airc.nist.gov/
AI_RMF_Knowledge_Base/Playbook); AI 600-1 Generative AI Profile (nvlpubs.nist.gov/nistpubs/ai/
NIST.AI.600-1.pdf).

**SOC 2 / AICPA TSC:** 2017 Trust Services Criteria (rev. 2022) (aicpa-cima.com — 2017 trust services
criteria with revised points of focus 2022).

**Microsoft (descriptive):** AGT Audit & Compliance — Merkle chain + HMAC signatures + inclusion
proofs, no documented replay (microsoft.github.io/agent-governance-toolkit/tutorials/
04-audit-and-compliance/); Azure AI Foundry observability — OTEL spans, App Insights default 90-day,
no documented replay (learn.microsoft.com/azure/foundry/concepts/observability); Azure Confidential
Ledger — signed offline-verifiable Merkle receipts but cloud-locked, not agent-aware
(learn.microsoft.com/azure/confidential-ledger/faq).

---

**Review checklist before external circulation:** (1) confirm provider-vs-deployer framing matches
target users; (2) legal review of all article/criterion citations; (3) re-verify Microsoft feature
descriptions against the latest AGT/Foundry releases (signing model + any new replay/signature
features can change); (4) confirm Akmon flag names (`--require-capture full`, `--require-signature`,
`--verify-key`) against the shipped CLI.
