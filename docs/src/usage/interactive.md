# Interactive mode

Interactive mode is the default way to work with Akmon when you want close control over prompts, permissions, and step-by-step execution.

```bash
akmon chat
```

## What the UI is showing you

The TUI is designed around operational awareness:

- conversation transcript and tool calls,
- approval prompts for side effects,
- session/provider/model identity,
- context/token/cache/cost signals.

It is not just chat; it is a control surface for autonomous execution.

## Typical interaction pattern

1. give focused task,
2. review tool calls and approvals,
3. inspect diffs before writes,
4. run verification commands,
5. iterate until completion.

Example starting prompt:

```text
Add input validation to user registration, update tests, and run verification commands after each file change.
```

## Status and context indicators

Key footer/top indicators usually include:

- session id,
- model/provider,
- cumulative input/output tokens,
- cache read tokens,
- cost estimate,
- context usage bar/percentage.

For long runs, monitor context percentage and compact/reset before quality drifts.

## Slash commands that matter most

- `/model` switch model mid-session,
- `/plan` create plan-only turn,
- `/context` view context budget and thresholds,
- `/cost` inspect usage/cost breakdown,
- `/copy` copy latest assistant response.

## Approval flow

When the model requests writes or command execution:

1. inspect proposed action/diff,
2. approve once or for session where appropriate,
3. deny if scope drifts.

Use session-wide allowances carefully; they trade control for speed.

## Common mistakes and troubleshooting

- **Mistake:** broad vague prompts ("fix everything").
  - **Fix:** split by subsystem and expected verification.
- **Mistake:** ignoring context/cost indicators in long sessions.
  - **Fix:** use `/context` and continue in focused phases.
- **Mistake:** approving shell writes blindly.
  - **Fix:** check command intent and command scope before allow.

See also [slash commands](../reference/slash-commands.md), [plan mode](./plan-mode.md), and [headless mode](./headless.md).
