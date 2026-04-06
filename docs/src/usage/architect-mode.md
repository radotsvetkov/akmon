# Architect Mode

Two-phase workflow: a **planner** model produces a plan, then your **main** model implements it.

## CLI

```bash
akmon --architect \
  --planner-model llama3.2 \
  --model claude-haiku-4-5-20251001 \
  --task "refactor the database layer to use connection pooling"
```

- **`--planner-model`** — cheap or fast model for the plan (default often `llama3.2` in config).
- **`--model`** — main model for implementation.

Planner output is captured and passed to the implementation phase. Plans can be saved under `.akmon/plans/` like plan mode.

## When to use it

- Large refactors where a written plan reduces wasted edits.
- When you want a smaller model to outline steps before spending tokens on a frontier model.

## Compared to plan mode

| | Plan mode (`--plan`) | Architect (`--architect`) |
| --- | --- | --- |
| Goal | Read-only analysis + saved plan | Plan then **run** implementation |
| Tools in plan phase | Read/search only | Planner uses read-only tools; main run gets full tool set after |

## Configuration

`[architect]` in `~/.akmon/config.toml` can set the default planner model. See [Configuration](./../getting-started/configuration.md).
