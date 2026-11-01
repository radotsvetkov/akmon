# Compliance crosswalk

Documented for Akmon `3.0.0`. Regulatory status checked on 1 November 2026.

**This page is not legal advice.** Akmon helps you produce tamper-evident, signed, offline-verifiable evidence about AI agent sessions. It is not a certification, it does not decide whether your system is in scope of any law, and using it does not make anything compliant. Validate every use below with your own legal and compliance teams.

## What Akmon evidence is

A sealed `.akmon` bundle gives a third party four things they can check offline:

1. **Integrity**: the record has not been altered since it was sealed.
2. **Signature**: which key sealed it (Ed25519, verifiable with `agef-verify` or OpenSSL 3).
3. **Operator attestation** (optional): which key claimed a named person or role approved it.
4. **Capture level**: how much of the session content was recorded (`full`, `partial`, `structural`).

It does **not** provide a trusted time (see [Trusted timestamps](../guides/trusted-timestamps.md)), key-to-person binding, retention, access control, or any judgement about whether the agent behaved correctly. See [What verification proves](./verifying-evidence.md).

## EU AI Act: what applies when

The AI Act is Regulation (EU) 2024/1689. It was amended by the **Digital Omnibus on AI**, Regulation (EU) 2026/1744, published in the Official Journal on 24 July 2026 and in force since 27 July 2026. The Omnibus moved the high-risk dates. They are deferred, not cancelled.

| Obligation | Applies from | Status on 1 Nov 2026 |
| --- | --- | --- |
| Prohibited practices (Art. 5), AI literacy (Art. 4) | 2 February 2025 | Applies |
| General-purpose AI model obligations | 2 August 2025 (Commission enforcement powers from 2 August 2026) | Applies |
| Penalties regime | 2 August 2025 | Applies |
| Transparency obligations (Art. 50) | 2 August 2026; grace period for marking until 2 December 2026 for systems already on the market | Applies |
| High-risk obligations for **Annex III** systems, including Art. 12, 19, 26 | **2 December 2027** | Not yet applicable |
| High-risk obligations for **Annex I** (product-embedded) systems | **2 August 2028** | Not yet applicable |

AI agents are not a separate category under the AI Act. The AI Office's FAQ (non-binding, preliminary) treats an agent like any other AI system: whether obligations apply depends on its use and risk classification.

Sources: [AI Act Service Desk, Article 113](https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-113); [AI Act Service Desk FAQ](https://ai-act-service-desk.ec.europa.eu/en/faq); EUR-Lex texts of Regulation (EU) 2024/1689 and Regulation (EU) 2026/1744.

## EU AI Act crosswalk

| Obligation | What Akmon evidence helps with | What it does not cover |
| --- | --- | --- |
| **Art. 12, automatic logging** (high-risk; from 2 Dec 2027 / 2 Aug 2028) | A hash-linked, signed record of each agent session that a market surveillance authority can verify offline. | Deciding what your system must log, logging over the system's whole lifetime, and anything outside the agent session. The specific minimum fields in **Art. 12(3)** apply only to remote biometric identification systems (Annex III point 1(a)). |
| **Art. 19, provider keeps logs ≥ 6 months** | Sealed bundles are portable files that can sit in immutable storage for as long as needed and still verify. | Retention itself, storage, access control, and deletion policy. See [Retention and archiving](../guides/retention.md). |
| **Art. 26(6), deployer keeps logs ≥ 6 months** | Same as Art. 19, for deployers. | Same as Art. 19. |
| **Art. 14, human oversight** | An operator attestation is key-backed evidence that a named role signed off on a specific session. It can help evidence oversight and sign-off. | Designing oversight measures, the competence of the overseer, and the ability to intervene. An attestation does not close Art. 14. |
| **Annex IV, technical documentation**, point 2(g): "test logs and all test reports dated and signed by the responsible persons" | A signed test-session bundle plus an operator attestation by the responsible person is a natural fit for signed test logs. | **Dated.** AGEF does not yet sign a time. Pair the bundle with an RFC 3161 timestamp ([Trusted timestamps](../guides/trusted-timestamps.md)). The rest of Annex IV is documentation Akmon does not produce. |
| **Art. 72, post-market monitoring** | Sealed sessions from production can feed a monitoring plan as verifiable samples. | The plan, the template (still pending from the Commission), and analysis. |
| **Art. 73, serious-incident reporting** (15 / 10 / 2 days; Art. 73(6): no investigation that alters the AI system in a way that could affect later evaluation of the causes before informing the authorities) | A bundle sealed at the time of the incident, plus an RFC 3161 timestamp taken at the time ([Trusted timestamps](../guides/trusted-timestamps.md)), helps show the session records were preserved. A signature alone does not prove when a bundle was sealed. | Detection, classification, the report itself, and the deadlines. Not altering the AI system (Art. 73(6)) is an operational duty Akmon does not address. Commission guidance on Art. 73 is still a draft. |
| **Art. 50, transparency** | Not directly. Akmon does not mark or label AI-generated content. | All of it. |

Sources: AI Act Service Desk pages for [Art. 12](https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-12), [Art. 19](https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-19), [Art. 26](https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-26), [Art. 72](https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-72), [Art. 73](https://ai-act-service-desk.ec.europa.eu/en/ai-act/article-73); [Annex IV](https://artificialintelligenceact.eu/annex/4/).

## Adjacent EU rules that already apply

| Rule | Logging requirement | What Akmon evidence helps with | What it does not cover |
| --- | --- | --- | --- |
| **DORA**, RTS (EU) 2024/1774, Art. 12 (financial entities) | ICT logs that are tamper-protected, time-synchronised, and retained for a defined period. | Tamper protection of agent-session records, verifiable by a third party. | Time synchronisation and trusted time, retention periods, coverage of non-agent ICT systems. |
| **NIS2**, Implementing Regulation (EU) 2024/2690, section 3.2 (applies to specific entity types: DNS, TLD, cloud, data-centre and CDN providers, managed (security) service providers, online marketplaces, search engines, social networks and trust service providers; other NIS2 entities get logging duties through national Art. 21 measures) | Logging and monitoring of relevant activity, with protected and retained logs. | Integrity-protected records of agent activity. | Monitoring, alerting, and everything outside agent sessions. |
| **Cyber Resilience Act** (reporting since 11 September 2026) | Reporting of actively exploited vulnerabilities and severe incidents. | A sealed record of an agent's involvement in an incident. | The reporting process. |
| **eIDAS**, Art. 41 and Implementing Regulation (EU) 2025/1929 | Qualified electronic time stamps (RFC 3161 profile, ETSI EN 319 422) carry a legal presumption of accuracy of time. | Akmon's signed statement can be timestamped by a qualified TSA today with the [interim recipe](../guides/trusted-timestamps.md). | Native timestamp embedding in AGEF is planned, not shipped. |

Sources: [RTS (EU) 2024/1774](https://eur-lex.europa.eu/eli/reg_del/2024/1774/oj); [IR (EU) 2024/2690](https://eur-lex.europa.eu/eli/reg_impl/2024/2690/oj); [CRA reporting](https://digital-strategy.ec.europa.eu/en/policies/cra-reporting); [IR (EU) 2025/1929](https://eur-lex.europa.eu/eli/reg_impl/2025/1929/oj/eng).

## Standards status

No harmonised standard gives a presumption of conformity for AI Act Art. 12 logging yet.

| Standard | Topic | Status |
| --- | --- | --- |
| ISO/IEC 24970 | AI system logging | Final draft (FDIS) |
| prEN 18229-1 | AI trustworthiness framework, part 1: logging | CEN Enquiry (draft) |
| EN 18286 | Quality management system for the AI Act | Published 2026 |

When ISO/IEC 24970 and prEN 18229-1 are final, compare their required fields with what your sealed sessions contain.

## Other frameworks

| Framework | What Akmon evidence helps with | What it does not cover |
| --- | --- | --- |
| **NIST AI RMF**, MEASURE 2.8 (transparency and accountability) and related traceability practices | A verifiable, traceable record of agent sessions with an honest capture level. | The risk measurement programme, thresholds, and conclusions. The AI RMF is voluntary. |
| **SOC 2**, CC7.x (system operations, monitoring) and CC8.1 (change management) | Signed evidence of agent-driven activity and changes, with optional operator sign-off. | Control design and operating effectiveness, completeness of monitoring, key management. Whether an auditor accepts the evidence is decided in your audit. |

## Practical guidance

- Seal sessions **when they happen**, not when someone asks. A bundle sealed months later proves less about the past.
- Timestamp the signed statement with an RFC 3161 authority (a qualified one for EU legal weight) when time matters.
- Keep bundles in immutable storage for at least six months once Art. 19/26(6) apply, and longer where sector rules or contracts require. Never delete agent logs on a short default schedule.
- Record which signing key belongs to which team, and keep operator keys personal.
- Use `--require-capture full` for evidence that must contain message content, and say plainly when a record is `structural`.

## See also

- [Trust model](./trust-model.md)
- [What verification proves](./verifying-evidence.md)
- [Retention and archiving](../guides/retention.md)
- [Trusted timestamps](../guides/trusted-timestamps.md)
- [Operator sign-off](../use-cases/operator-sign-off.md)
