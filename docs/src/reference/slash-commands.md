# Slash Commands

Documented for Akmon `2.0.0`.

In `akmon chat`, type `/` then command name. Use `/help` in-session as runtime source of truth.

## Session & navigation

| Command | Description |
| --- | --- |
| `/help` | Show command list |
| `/exit` (`/quit`, `/q`) | Save and exit |
| `/clear` | Clear UI + chat context (`--hard` also clears spec markdown cache) |
| `/reset` | Start a new session (saves current first) |
| `/sessions` | Session picker |
| `/resume <id-prefix>` | Resume by session ID prefix |

## Project memory

| Command | Description |
| --- | --- |
| `/init` | Generate or refresh `AKMON.md` |
| `/import` | Import external tool context |
| `/export` | Export `AKMON.md` to another format |
| `/update-context` | Open `AKMON.md` in `$EDITOR` and reload |
| `/new <name>` | Scaffold a new project in current directory |

## Models

| Command | Description |
| --- | --- |
| `/model` | Show/set model for next turns |
| `/models` | Alias for `/model` |
| `/architect` | Next message uses planner then main model |

## Planning & specs

| Command | Description |
| --- | --- |
| `/plan` | Next message runs in read-only plan mode |
| `/implement` | Run the last captured plan |
| `/edit-plan` | Edit latest plan in `$EDITOR` |
| `/view-plan` | View latest plan in overlay |
| `/spec` | List feature specs under `.akmon/specs` |

## Insight & diagnostics

| Command | Description |
| --- | --- |
| `/cost` | Token/cost summary |
| `/audit` | Session audit log view |
| `/context` | Context-window usage breakdown |
| `/config` | Settings UI for model estimates |
| `/index` | Semantic index status |
| `/doctor` | Provider key/status summary |
| `/mcp` | MCP setup hints and configured servers |
| `/copy` | Copy last assistant response |
| `/transcript` (`/export-chat`) | Export chat to `.akmon/transcript_export.md` |

## Verification

Run `akmon chat`, then `/help`, and verify command list contains the expected set for your build.
