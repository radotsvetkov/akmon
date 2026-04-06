# `AKMON.md` reference

`AKMON.md` lives at the **project root**. It is the **source of truth** for how Akmon should behave in this repo.

## Typical sections

- **Product** — what the software is for, users, constraints.
- **Architecture** — main crates/modules, data flow, external systems.
- **Conventions** — formatting, error handling, testing expectations.
- **Current sprint** — what you are working on now (keep this fresh).
- **Done** — recently completed decisions (optional).

Exact template evolves with `akmon init`; always review generated output.

## Keeping it current

1. Edit `AKMON.md` in your editor, or use **`/update-context`** in the TUI to open `$EDITOR` and reload after save.
2. After major refactors, run **`/init`** again or ask Akmon to update the file.

## Exporting to other tools

[`akmon export`](./export.md) writes derived files (`CLAUDE.md`, `AGENTS.md`, …) with a header reminding readers to edit `AKMON.md` instead.

## Related

- [Import](import.md) — build `AKMON.md` from other tools’ context files.
- [Project intelligence](../languages/rust.md) — language-aware hints also augment prompts when detected.
