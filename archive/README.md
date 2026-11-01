# Archive (not built)

Akmon v3.0 narrows the product to one job: seal any AI agent session into a
signed, tamper-evident AGEF bundle that a stranger can verify offline. The code
below did not serve that job and was moved here so it stops shipping in the
binary. Nothing in this directory is compiled: the workspace sets
`exclude = ["archive"]` and no crate depends on these files.

The files keep their git history (`git log --follow archive/<path>`). The
complete pre-archive tree is tagged **`archive/pre-focus-v2.2.1`**. That tag
points at the same commit as the `v2.2.1` release tag, so `v2.2.1` works
equivalently in every command below (use it if your clone does not have the
`archive/` tag; fetch tags with `git fetch --tags`).

## How to restore an item

Restore the original file(s) from the tag, then re-add the wiring listed for
that item (module declarations, CLI dispatch, Cargo features):

```sh
git checkout archive/pre-focus-v2.2.1 -- <original path>
```

For example, to bring back `akmon slo`:

```sh
git checkout archive/pre-focus-v2.2.1 -- crates/akmon-cli/src/slo_cmd.rs \
  crates/akmon-core/src/slo.rs crates/akmon-core/src/slo_trend.rs
```

The quickest way to see all the wiring an item needed is to diff the tag
against the commit that archived it:
`git diff archive/pre-focus-v2.2.1 HEAD -- crates/`.

## Archived items

| Item | Archived files (original path) | Why |
|---|---|---|
| `akmon import` (synthesize AKMON.md from CLAUDE.md, AGENTS.md, .cursorrules, …) | `crates/akmon-cli/src/import_cmd.rs` → `archive/akmon-cli/src/import_cmd.rs` | Context-file sync is unrelated to evidence, and the name collided with `akmon bundle import` / `akmon otel import`. `akmon init` still reads existing context files (via `akmon_core::context_import`) as optional input for AKMON.md. |
| `akmon export` (copy AKMON.md into other tools' files) | `crates/akmon-cli/src/export_cmd.rs` → `archive/akmon-cli/src/export_cmd.rs` | Same as import. It also overwrote hand-written CLAUDE.md/AGENTS.md without `--force` (review finding F-4). |
| `akmon spec` (multi-phase spec workflow) | `crates/akmon-cli/src/spec_cmd.rs`, `crates/akmon-cli/src/cli_forward.rs` → `archive/akmon-cli/src/` | Agent workflow feature, not needed by the reference producer. It re-executed akmon with API keys on the child's argv and dropped the operator's policy flags (F-5, F-14). |
| `read_spec` / `write_spec` agent tools | `crates/akmon-tools/src/read_spec.rs`, `crates/akmon-tools/src/write_spec.rs` → `archive/akmon-tools/src/` | Only served the spec workflow. |
| `akmon scout` and the global `--dossier` flag | `crates/akmon-cli/src/scout_cmd.rs` → `archive/akmon-cli/src/scout_cmd.rs` | Context-dossier generator for the agent prompt; not part of the trust path. |
| `akmon slo verify` / `akmon slo trend`, `[slo]` config | `crates/akmon-cli/src/slo_cmd.rs`, `crates/akmon-core/src/slo.rs`, `crates/akmon-core/src/slo_trend.rs` → `archive/akmon-cli/src/`, `archive/akmon-core/src/` | Reliability guardrails over the unsigned legacy JSON artifacts. A legacy `[slo]` section in `~/.akmon/config.toml` is now ignored, not an error. `RunReliabilityMetrics` (in the run report) is kept. |
| Semantic index: `--index`, `semantic_search` tool, TUI `/index`, `semantic-index` cargo feature, `fastembed`/`ort` ML stack | `crates/akmon-index/` → `archive/crates/akmon-index/`; `crates/akmon-tools/src/semantic_search.rs` → `archive/akmon-tools/src/semantic_search.rs` | Large optional ML dependency stack (model download on first use) that only improved agent code search. Removing it shrinks the supply chain of a trust product. |
| `spawn_subagent` tool and `AKMON_EXPERIMENTAL_SUBAGENTS` | `crates/akmon-query/src/subagent_tool.rs` → `archive/akmon-query/src/subagent_tool.rs` | Multi-agent orchestration was already an explicit non-goal ("one agent, one session, one artifact") and was off by default. |
| `akmon sign` and the post-session `[signing]` hook (decision D-05) | `crates/akmon-cli/src/signing.rs` → `archive/akmon-cli/src/signing.rs` | Superseded by native `akmon bundle keygen` / `bundle sign` / `bundle verify --verify-key` (AGEF-SIG-v1, D-18). The hook signed the bare head outside the bundle, so no verifier could check it (TC-15), and the automatic post-run hook always failed on the journal lock (F-1). A legacy `[signing]` section is now ignored. |

## Deprecated, not archived

`akmon audit verify` and `akmon evidence verify` still work but are hidden from
`--help` and print a one-line deprecation notice pointing to
`akmon bundle verify`. The legacy audit JSONL and evidence JSON writers are kept
until a compatibility renderer exists.
