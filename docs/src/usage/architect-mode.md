# Architect mode

Architect mode runs a planner phase and implementation phase in one command.

## Why use architect mode

It is useful when you want:

- cheap/fast planning model,
- stronger implementation model,
- less manual handoff between plan and execution.

## Basic command

```bash
akmon --architect \
  --planner-model llama3.2 \
  --model claude-haiku-4-5-20251001 \
  --task "Refactor database layer to use connection pooling with migration-safe rollout"
```

## How the phases differ

| Phase | Model | Tool scope | Output |
| --- | --- | --- | --- |
| Planner | `--planner-model` | read-oriented analysis | ordered plan |
| Implementer | `--model` | full policy-checked tool set | code + verification |

## Practical model strategy

- use low-cost local/cloud model for planning,
- use Haiku/Sonnet-class model for implementation complexity,
- reserve expensive models for hard reasoning bottlenecks.

## Suggested usage pattern

1. run architect command,
2. inspect generated plan artifacts,
3. review first implementation diff before broad approvals,
4. continue in focused increments.

## Common mistakes and troubleshooting

- **Mistake:** planner model too weak to map architecture.
  - **Fix:** upgrade planner model for complex repos.
- **Mistake:** no budget cap in long implement phases.
  - **Fix:** combine with `--max-budget-usd`.
- **Mistake:** skipping post-plan review.
  - **Fix:** verify plan assumptions before writes.

Related: [plan mode](./plan-mode.md), [headless mode](./headless.md), [configuration](../getting-started/configuration.md).
