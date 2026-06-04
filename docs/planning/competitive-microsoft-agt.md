# Competitive analysis — Microsoft AI-agent governance vs. Akmon

**Status:** internal strategy / positioning reference (not normative). **Date:** 2026-06-05.
**Lens:** tamper-evident, content-addressed, merkle-linked, **offline-signed**, deterministically
**replayable**, **portable** evidence for regulated / EU-AI-Act buyers.

## Bottom line

Microsoft has shipped a strong governance **runtime** (the open-source Agent Governance Toolkit,
GA April 2026) and a strong tamper-evident **cloud ledger** (Azure Confidential Ledger). But **no
single Microsoft product** gives you a portable, self-contained, **asymmetrically signed (Ed25519)**,
**offline-verifiable-by-a-stranger-with-no-Microsoft-install**, **deterministically replayable**
evidence artifact that sits on top of *any* agent. Microsoft's tamper-evidence is either (a)
hash-chain-only with **no signatures and no standalone verifier** (the Toolkit), or (b)
cryptographically excellent but **Azure-cloud-locked and not agent-specific** (Confidential Ledger).
**That seam is Akmon's wedge** — and it is exactly what the D-18 signing work + the D-19 OTEL-import
pivot target.

## The offerings and their evidence architecture (sourced)

| Offering | Tamper-evident | Asymmetric **signature** | Standalone outside verifier (no vendor install) | Deterministic **replay** | Portable sealed artifact | Cloud-locked |
|---|---|---|---|---|---|---|
| **Agent Governance Toolkit (AGT)** | Yes (SHA-256 chain + Merkle) | **No** (HMAC only — shared secret) | **No** (must install the toolkit) | **No** | No (loose JSONL) | No (producer-agnostic) |
| **Azure Confidential Ledger** | Yes (Merkle + signed root) | **Yes** (signed receipts) | Partial (needs ledger signing cert from Azure) | No | No (opaque blobs) | **Yes** |
| **Microsoft Purview (AI/Copilot audit)** | Yes (hash-chained entries) | Not documented for offline use | **No** (verify only inside Purview; CSV export) | No | No | **Yes** |
| **Azure AI Foundry observability** | **No** (mutable telemetry) | No | No | **No** (MS docs: traces "cannot support full replay") | No (OTLP spans) | Mostly |

Primary sources: AGT audit spec `AUDIT-COMPLIANCE-1.0.md` and the audit tutorial
(github.com/microsoft/agent-governance-toolkit; microsoft.github.io/agent-governance-toolkit) — the
spec is hash-linking only (no asymmetric crypto), the tutorial confirms HMAC + "no standalone CLI
verifier," and §14.4 states external anchoring is **not** performed. Azure Confidential Ledger:
learn.microsoft.com/azure/confidential-ledger (overview + verify-write-transaction-receipts).
Purview: learn.microsoft.com/purview/ai-microsoft-purview, /audit-copilot, /audit-log-export-records.
Foundry replay caveat: learn.microsoft.com/azure/ai-foundry/observability/concepts/trace-agent-concept.
EU AI Act logging trigger (Art. 12 + Annex IV; enforcement Aug 2, 2026): helpnetsecurity.com
"EU AI Act logging requirements" (2026-04-16).

## Where Akmon wins (each strength → the specific Microsoft gap)

1. **Offline Ed25519-signed session head (asymmetric non-repudiation)** — AGT has *no* signatures, only
   a hash chain + HMAC; a hash chain proves a log is self-consistent, a signature proves *who sealed it*.
   **Shipped (D-18).** This is the single sharpest technical gap.
2. **Tiny standalone `agef-verify` + portable `.akmon` bundle** — verify integrity *and* signature on an
   air-gapped laptop with no Akmon/Microsoft install and zero trust in the producer. AGT needs its toolkit
   installed; Purview/Foundry/Confidential-Ledger need Azure. **Shipped (D-18 + Item 4.3).**
3. **OTEL-GenAI import → sits on top of any agent, then replays it** — ingest the exact OTEL spans Foundry/
   Agent Framework already emit and turn unsealed telemetry into a signed, portable, replayable bundle.
   Microsoft explicitly does **not** replay. **In build (D-19, Item 9.1).** This weaponizes Microsoft's own
   emission standard against its own gap.

## Where Microsoft is stronger — do not pretend to compete

- **Distribution.** Purview/Copilot Control System/Agent 365 are already in every M365 tenant. Position
  Akmon as **complementary** ("seal what Purview captures," "export-and-verify what Foundry traces"), not
  as a replacement governance plane.
- **Azure Confidential Ledger genuinely does signed, offline-verifiable Merkle receipts.** Be honest: the
  crypto primitive exists at Microsoft — Akmon's win is **packaging + portability + cloud-independence +
  agent-awareness + replay**, not "Microsoft can't do crypto."
- **Ecosystem & standards weight.** Foundry's first-party tracing across frameworks, a built-in evaluator
  library, co-driving OTEL-GenAI, and enterprise compliance attestations at a scale a startup can't match.
  **Ride the standard; consume their telemetry; don't fight the policy-engine/eval bake-off.**
- **Inline real-time enforcement.** AGT's sub-0.1 ms policy kernel is an *enforcement* layer; Akmon is an
  *evidence* layer. Different layer — don't get drawn into a gatekeeper comparison.

## Honesty caveats (carry into any public comparison)

- AGT **is** tamper-evident (hash chain + Merkle). Do not say it "has no integrity." Its gaps are: no
  asymmetric signatures, no outside-runnable standalone verifier, no replay, no portable sealed bundle,
  anchoring is a no-op.
- "Rust" alone is **not** a differentiator — AGT ships a Rust SDK too. The differentiators are the AGEF
  *format* + the signed head + the standalone verifier + replay + producer-agnostic OTEL import.

## One-line positioning

> Microsoft can capture and even tamper-proof your agent logs — but only inside Azure, only as opaque
> rows, never as a **signed, portable, replayable artifact a regulator can verify offline**. Akmon is
> that artifact, on top of *any* agent.

## Implications for the plan

- The D-18 signing work and the standalone `agef-verify` are confirmed as the load-bearing wedge — keep
  them front-and-center in positioning (Item 9.6 docs/site rewrite).
- **Item 9.4 (compliance crosswalk)** should explicitly map AGEF evidence fields ↔ **EU AI Act Art. 12 +
  Annex IV** logging obligations — that is the buying trigger and the language regulated buyers use.
- The OTEL-GenAI importer (Item 9.1) should treat **Azure AI Foundry / Agent Framework** OTEL output as a
  first-class input shape (it is a concrete, named source of traces to seal).
