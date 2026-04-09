# Multi-agent setups & automation

“Multi-agent” here means **multiple Akmon processes or roles cooperating**—not a single monolithic chat. Akmon is a **CLI**: you compose it with shells, schedulers, CI, and other agents.

## Headless JSON for glue code

Use `--output json` with `--yes` so scripts get structured errors and results:

```bash
akmon --yes --output json --task "Summarize TODO comments in src/" | jq .
```

If configuration is invalid (missing API key, bad model name), recent releases surface a **JSON error on stdout** in JSON mode so parsers do not have to scrape stderr.

## Pattern: planner + implementer (single machine)

1. **Planner** (fast/local model):

   ```bash
   akmon --plan --task "Break down feature X" --model llama3.2
   ```

2. Human or script reviews `.akmon/plans/*.md`.

3. **Implementer** (stronger model):

   ```bash
   akmon --yes --task "Implement the plan in .akmon/plans/ for feature X" \
     --model claude-haiku-4-5-20251001
   ```

For built-in two-phase routing, see [Architect mode](../usage/architect-mode.md) (`--architect`, `--planner-model`).

## Pattern: CI agent

Typical GitHub Actions job:

1. Checkout
2. Install Akmon binary from Releases
3. Export provider key as secret
4. Run `akmon --yes --task "run tests and fix obvious failures"` with a **narrow** task scope, or only static checks

Keep CI tasks **read-biased**: rely on `--yes` auto-approving reads; writes still prompt unless you use dedicated automation branches and review policies.

## Pattern: audit trail across runs

Every session can write JSONL under `.akmon/audit/`. For compliance:

- Treat the audit directory as **evidence**, not scratch space—back it up or export to your log stack.
- Use consistent repo roots so paths in the log stay stable.

See [Audit log](../features/audit-log.md).

## Integrating with other tools

- **MCP**: expose extra tools via [MCP Tools](../features/mcp.md); combine with IDE-hosted MCP servers carefully (trust boundary).
- **Import/export**: sync `AKMON.md` with Claude Code, Cursor, Codex, etc. ([Import](../project/import.md), [Export](../project/export.md)).
- **Semantic search**: optional indexing for large repos ([Semantic search](../features/semantic-search.md)).

## Scaling concerns

| Concern | Approach |
| --- | --- |
| Cost | Use plan mode first; local Ollama for exploration; watch cache/token lines in the TUI |
| Reliability | Narrow `--task` strings; avoid “fix everything” in one headless shot |
| Safety | Default policy still confirms writes; use allowlists for repeated shell patterns |
| Observability | JSON output + JSONL audit + session summaries |

When multiple humans **and** automations touch the same repo, standardize `AKMON.md` **Current sprint** so every agent run shares the same intent.
