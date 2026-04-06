# Slash Commands

In **`akmon chat`**, type `/` then a command name. **Tab** may complete available commands.

## Session & navigation

| Command | Description |
| --- | --- |
| `/help` | List commands / open help overlay |
| `/exit` | Quit with summary (also Ctrl+D / idle Ctrl+C) |
| `/clear` | Clear transcript context |
| `/new` | New session in same directory |
| `/sessions` | Session picker |
| `/resume` | Resume by id or picker |

## Project memory

| Command | Description |
| --- | --- |
| `/init` | Generate or refresh `AKMON.md` |
| `/import` | Import external tool context |
| `/export` | Export `AKMON.md` to another format |
| `/update-context` | Open `AKMON.md` in `$EDITOR` and reload |

## Models

| Command | Description |
| --- | --- |
| `/model` | Interactive picker |
| `/model <id>` | Jump directly |

## Planning & specs

| Command | Description |
| --- | --- |
| `/plan` | Next message runs in read-only plan mode |
| `/implement` | Run the last captured plan |
| `/edit-plan` | Edit latest plan in `$EDITOR` |
| `/view-plan` | Show plan snippet in TUI |
| `/spec` | Spec workflow helpers |

## Insight

| Command | Description |
| --- | --- |
| `/cost` | Token / cost overlay |
| `/audit` | Audit log overlay |
| `/audit` paths | (see CLI) |

Exact sets may expand — `/help` inside the app is source of truth.
