# Plan Mode

Plan mode analyzes your codebase and produces a written
implementation plan without touching any files (write tools are not registered).

```bash
akmon --plan --task "your task"
```

## How it works

In plan mode:

- Only **read** tools are available (read, search, list; semantic search when `--index`).
- **Write** tools are absent from the tool registry — not merely disabled.
- The model produces a detailed plan.
- Plans are saved under **`.akmon/plans/`** when persistence succeeds.

## Using plan mode

```bash
# Generate a plan
akmon --plan \
  --model claude-haiku-4-5-20251001 \
  --task "add rate limiting to all API endpoints"
```

Typical CLI footer after a successful save:

```
Plan saved to .akmon/plans/...

Review:  cat path/to/plan.md
Edit:    $EDITOR path/to/plan.md
Implement: akmon --task 'implement the plan in ...'
```

## In the TUI

```
/plan
```

Then send your task message. Use **`/implement`** to run the stored plan, **`/edit-plan`** to open it in `$EDITOR`, or **`/view-plan`** to preview in the UI.

## Why plan first?

Planning up front reduces thrash: the model maps the full scope before edits land in your tree. Pair with [Architect mode](./architect-mode.md) when you want an automatic plan→implement pipeline.
