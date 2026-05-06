# Regulated Reviewer Flow

Documented for Akmon `2.0.0`.

## Who this is for

Reviewers, tech leads, and compliance engineers validating AI-assisted code-change sessions.

## What you will have at the end

- A single repeatable checklist from run output to verification-ready handoff.

## Prerequisites

1. A completed Akmon run with session ID.
2. Access to repository artifacts (`.akmon/audit`, `.akmon/evidence`).

## Steps

1. Capture the session ID from run output.

```bash
SESSION_ID="<session-uuid>"
```

2. Verify audit chain and evidence artifact linkage.

```bash
akmon audit verify ".akmon/audit/${SESSION_ID}.jsonl"
akmon evidence verify ".akmon/evidence/${SESSION_ID}.json"
```

3. Verify session integrity at journal level.

```bash
akmon verify "${SESSION_ID}"
```

4. Replay for behavioral divergence checks when required.

```bash
akmon replay "${SESSION_ID}" --format json | tee replay.json
```

5. Export portable bundle for external review or archive retention.

```bash
akmon bundle export "${SESSION_ID}" --output "${SESSION_ID}.akmon"
```

6. If sensitive content must be removed, create derivative redacted bundle.

```bash
akmon redact "${SESSION_ID}" \
  --output "${SESSION_ID}-sanitized.akmon" \
  --object <object-hash> \
  --reason "compliance redaction"
```

## Verification

A handoff is review-ready when:
- all verification commands exit `0`,
- replay report is pass or divergences are explicitly accepted,
- exported bundle is present and, if redacted, passes `bundle import --verify-only`.

## Troubleshooting

- If `verify` fails, stop release review and inspect violation category before proceeding.
- If `replay` diverges, treat as change-detection signal and triage expected vs unexpected drift.
- If bundle import verify-only fails, do not distribute the bundle externally.
