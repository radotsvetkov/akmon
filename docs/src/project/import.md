# Importing Context

If you have been using another AI coding tool, your project
may already have context files. Akmon can **synthesize** them into `AKMON.md`.

## Supported tools

| Tool | Context files |
|---|---|
| Claude Code | `CLAUDE.md`, `.claude/CLAUDE.md` |
| Codex / OpenCode | `AGENTS.md` |
| Cursor | `.cursorrules`, `.cursor/rules/*.mdc` |
| Gemini CLI | `GEMINI.md` |
| Kiro | `.kiro/steering/*.md`, `.kiro/specs/` |
| Windsurf | `.windsurfrules`, `.windsurf/rules/` |
| GitHub Copilot | `.github/copilot-instructions.md` |
| Cline / RooCode | `.clinerules`, `.roo/rules/` |
| Aider | `.aider.conf.yml` |
| Generic | `AGENTS.md`, `llms.txt` |

## Basic usage

```bash
cd your-project
akmon import
```

Akmon scans context files and uses your configured model to build **`AKMON.md`**.

## Preview without writing

```bash
akmon import --dry-run
```

## Import from a specific tool only

```bash
akmon import --from claude-code
akmon import --from cursor
akmon import --from kiro
```

## Overwrite existing AKMON.md

```bash
akmon import --force
```

## In the TUI

When no `AKMON.md` exists, the welcome screen may suggest **`/import`**. Run it to perform the same synthesis from inside `akmon chat`.

## See also

- [Export](./export.md)
- [AKMON.md reference](./akmon-md.md)
