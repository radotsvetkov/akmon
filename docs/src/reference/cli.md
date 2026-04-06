# CLI Reference

## Synopsis

```
akmon [FLAGS] [SUBCOMMAND]
```

Flags vary by release — use `akmon --help` for the authoritative list.

## Frequently used flags

| Flag | Purpose |
| --- | --- |
| `--model` | Model id for the session / task |
| `--task` | Headless task string |
| `--yes` | Auto-approve safe reads |
| `--yes-web` | Auto-approve web fetch where policy allows |
| `--shell-allow` | Glob pattern for permitted shell (repeatable) |
| `--output` | `text` or `json` |
| `--plan` | Plan-only mode |
| `--architect` | Planner + implementer pipeline |
| `--planner-model` | Planner model for architect mode |
| `--auto-commit` | Commit after writes (git tool) |
| `--index` | Enable semantic index features when built in |

## Provider flags

Examples:

```
--anthropic-key
--openrouter-key
--openai-key
--groq-key
--azure-endpoint / --azure-key / --azure-api-version
--bedrock
--aws-region
--openai-compatible-url / --openai-compatible-key
```

Environment variables usually mirror these; see [Environment variables](./env-vars.md).

## Subcommands (typical)

| Command | Role |
| --- | --- |
| `akmon chat [DIR]` | Interactive TUI |
| `akmon init` | Generate / refresh `AKMON.md` |
| `akmon new` | Scaffold project |
| `akmon import` | Synthesize `AKMON.md` from other tools |
| `akmon export` | Export `AKMON.md` to other formats |
| `akmon spec` | Spec workflow phases |
| `akmon config` | Config management |

```bash
akmon chat
akmon --plan --task "describe module boundaries"
akmon --yes --output json --task "list TODOs" | jq .
akmon import --dry-run
akmon export --all
```

Subcommand details: `akmon <cmd> --help`.
