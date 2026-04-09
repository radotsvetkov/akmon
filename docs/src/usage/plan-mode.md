# Plan mode

Plan mode performs read-only analysis and produces implementation plans without changing files.

```bash
akmon --plan --task "your task"
```

## Why plan mode exists

Large tasks fail when implementation starts before scope is understood. Plan mode separates discovery from execution:

1. map relevant files and constraints,
2. produce ordered implementation steps,
3. define verification per step,
4. execute later with lower risk.

## What is allowed in plan mode

- read/list/search tools,
- optional semantic search when enabled,
- no write/edit/patch tool registration.

This is structural read-only behavior, not just "please don't write."

## Recommended workflow

```bash
akmon --plan \
  --model claude-haiku-4-5-20251001 \
  --task "Design migration from sqlite auth sessions to redis-backed sessions with rollback strategy"
```

Then:

```bash
ls .akmon/plans
$EDITOR .akmon/plans/<latest>.md
akmon --task "Implement the approved plan in .akmon/plans/<latest>.md step by step"
```

## What a good plan should contain

- target files/modules,
- ordered steps,
- risk notes and migration impact,
- verification commands after each step,
- rollback hints.

## TUI usage

- run `/plan`,
- submit task,
- review plan,
- run `/implement` when approved.

## Common mistakes and troubleshooting

- **Mistake:** skipping plan review before implementation.
- **Mistake:** one giant implementation step instead of checkpoints.
- **Mistake:** missing verification commands in plan.

Plan mode pairs naturally with [architect mode](./architect-mode.md) and [spec workflow](./spec-workflow.md).
