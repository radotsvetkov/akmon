# Documentation Maintainers Guide

## Scope

This guide defines how to keep Akmon documentation accurate for regulated engineering users.

## Source of truth rule

For CLI/config/reference pages:

1. Validate command and flag behavior from `crates/akmon-cli/src/main.rs` and command modules in `crates/akmon-cli/src/*_cmd.rs`.
2. Validate config keys from `crates/akmon-config/src/lib.rs`.
3. Validate provider resolution behavior from `crates/akmon-models/src/llm_connect.rs`.
4. Only use `docs/src/releases/*.md` for release narrative; never as authoritative CLI/config truth.

If code and docs disagree, update docs or open a mismatch note with exact paths/snippets.

## Update workflow when CLI changes

1. Run `akmon --help` and `akmon <subcommand> --help` for changed command areas.
2. Update relevant pages under `docs/src/reference/`, `docs/src/getting-started/`, and `docs/src/tutorials/`.
3. Ensure each procedural page has:
   - Who this is for
   - What you will have at the end
   - Prerequisites
   - Steps
   - Verification
   - Troubleshooting
4. Update navigation in `docs/src/SUMMARY.md` if journey order changes.
5. Run docs checks:

```bash
bash scripts/docs/run_all.sh
```

6. Fix all reported links/structure issues before merge.

## Quarterly docs audit checklist

- Verify `Documented for Akmon X.Y.Z` markers match current workspace version.
- Re-check every `docs/src/reference/*.md` page against CLI/config Rust sources.
- Confirm tutorials still produce expected artifact paths under `.akmon/`.
- Confirm policy/evidence/verify/replay pages align with fail-closed integrity model.
- Remove or relabel any roadmap-only claims that are not implemented.
- Re-run `bash scripts/docs/run_all.sh` on a clean working tree.

## Tutorial definition of done

A tutorial is done only when all are true:

1. Commands are copy-pasteable and match actual binary flags.
2. Prerequisites are explicit (repo state, provider creds, expected files).
3. Includes expected result after major steps.
4. Includes evidence-specific sections:
   - What gets recorded
   - How a reviewer validates it
5. Includes at least one failure mode and recovery path.
6. Passes docs CI checks locally.
