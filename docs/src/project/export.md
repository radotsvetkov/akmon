# Exporting Context

Export **`AKMON.md`** to native formats for other AI tools, useful for teams using mixed workflows.

## Export to all tools

```bash
akmon export --all
```

Typical outputs include `CLAUDE.md`, `AGENTS.md`, `.cursor/rules/akmon.mdc`, `.kiro/steering/akmon.md`, Copilot instructions, Windsurf rules, Cline rules, etc. (exact set follows the CLI help for your version).

## Export to a specific tool

```bash
akmon export --tool claude-code   # writes CLAUDE.md
akmon export --tool codex         # writes AGENTS.md
akmon export --tool cursor        # writes .cursor/rules/akmon.mdc
akmon export --tool kiro          # writes .kiro/steering/akmon.md
akmon export --tool gemini        # writes GEMINI.md
akmon export --tool copilot       # writes .github/copilot-instructions.md
akmon export --tool windsurf      # writes .windsurfrules
akmon export --tool cline         # writes .clinerules
```

## Preview without writing

```bash
akmon export --all --dry-run
```

## Workflow for multi-tool teams

1. Maintain **`AKmon.md`** as the single source of truth.
2. Run `akmon export --all` after meaningful updates.
3. Commit exports alongside `AKMON.md` if your team wants them in-repo.

Exported files should carry a banner like:

```text
<!-- Generated from AKMON.md by Akmon -->
<!-- Edit AKMON.md, then run: akmon export --tool claude-code -->
```

## See also

- [Import](./import.md)
