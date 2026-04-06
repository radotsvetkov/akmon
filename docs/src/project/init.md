# `akmon init`

Analyzes your project and generates **`AKMON.md`** — structured project memory used in every session.

## Usage

```bash
cd your-project
akmon init
```

- Detects stack (languages, frameworks, tooling).
- Writes or updates `AKMON.md` with product context, architecture, conventions, and sprint sections.
- If other tools already left context files (`CLAUDE.md`, `.cursorrules`, …), you can run [`akmon import`](./import.md) first to synthesize them into `AKMON.md`.

## Why run it

Sessions with `AKMON.md` get better, more consistent answers because the model sees your conventions upfront.

## TUI

In `akmon chat`:

```
/init
```

Same operation from inside the interactive UI.

## See also

- [AKMON.md reference](./akmon-md.md)
- [Importing context](./import.md)
