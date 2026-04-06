# Interactive Mode

```bash
akmon chat
```

Opens the terminal TUI — a full-screen interface for conversational
AI-assisted development.

## Interface layout

```
┌─ akmon · v1.5.x  │  INTERACTIVE ─────────────────────────────────────┐
│                                                                        │
│  You: find where authentication tokens are validated                   │
│                                                                        │
│  → semantic_search                                                     │
│  ✓ semantic_search  "token validation"          [Tab to expand]        │
│  → read_file                                                           │
│  ✓ read_file  src/auth/middleware.rs                                   │
│                                                                        │
│  Akmon: Token validation happens in src/auth/middleware.rs.            │
│  The `validate_jwt` function on line 47 decodes the Bearer             │
│  token using the HS256 algorithm and checks expiry...                  │
│                                                                        │
├─ cwd · model · provider ────────────────────────────────────────────────┤
├─ session · tokens · cache · ~$cost · step ────────────────────────────┤
│  ↳ context: file1  file2  +N more                                       │
│ > type a message or / for commands                                     │
└────────────────────────────────────────────────────────────────────────┘
```

(Layout evolves between versions; two-line status bar, context row, and diff confirmations are typical.)

## Status bar

The status area shows:

- **Session ID** (short prefix) — matches audit log filename
- **Tokens** — cumulative input/output usage for the session
- **Cache** — prompt cache read tokens when using providers that support it (green when non-zero)
- **Cost** — heuristic USD estimate when not on a free local profile
- **Step** — current agent step when a turn is running

A **top status line** usually shows shortened **working directory**, **model**, and **provider** name.

## Context bar

When Akmon has read or written files, a context line may appear above
the input showing active paths (e.g. first two basenames plus `+N more`).

## Tool cards

Each tool call appears as a card. Press **Tab** to expand:

```
✓ read_file  src/auth/middleware.rs    [Tab to expand]
```

Expanded (example):

```
✓ read_file  src/auth/middleware.rs
  args: { "path": "..." }
  result: ...
```

## Confirmation prompts

Before file writes, Akmon shows a **unified diff** preview. Approve or deny with **`y`** / **`n`** (or **`N`**) as prompted.

## Starting a session in a specific directory

```bash
akmon chat /path/to/project
# or
cd /path/to/project && akmon chat
```

## Switching models mid-session

```
/model
```

Opens a picker showing available models by provider. Choose a row and confirm.

See also [Slash commands](../reference/slash-commands.md).
