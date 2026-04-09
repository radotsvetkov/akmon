# Tutorials overview

These tutorials assume you have [installed](../getting-started/installation.md) Akmon and completed [Quick start](../getting-started/quickstart.md) or run `akmon config` once.

| Tutorial | You will |
| --- | --- |
| [Step-by-step by language](./step-by-step.md) | Walk through the same “first hour” workflow in Rust, Go, Python (Flask & FastAPI), and Elixir |
| [Multi-agent & automation](./multi-agent-automation.md) | Combine headless mode, JSON output, CI, and orchestration patterns |
| [Architecture patterns](./architecture-patterns.md) | Choose planner/implementer splits, plan mode, spec workflow, and documentation strategy |

## Recommended order

1. Pick your stack in [Step-by-step](./step-by-step.md) and run the **same** high-level task: generate `AKMON.md`, run one `--plan` pass, then one `--yes` implementation pass.
2. Read [Multi-agent & automation](./multi-agent-automation.md) before wiring Akmon into cron, GitHub Actions, or multi-repo scripts.
3. Use [Architecture patterns](./architecture-patterns.md) when a single interactive session is not enough—e.g. large refactors, regulated audit trails, or team-wide conventions.

Related book sections: [Language guides](../languages/rust.md), [Plan mode](../usage/plan-mode.md), [Architect mode](../usage/architect-mode.md), [Headless mode](../usage/headless.md).
