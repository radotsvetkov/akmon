# Git integration

Akmon uses git context to improve planning and verification, and can perform git operations under policy controls.

## What git-aware workflows unlock

- better change understanding (`diff`, `log`, `status`),
- safer review loops (small commits per step),
- easier rollback when automation goes wrong.

## Operation classes

| Class | Examples | Typical approval posture |
| --- | --- | --- |
| Read-only | `status`, `diff`, `log`, `show` | often auto-approved in `--yes` mode |
| Mutating | `add`, `commit`, `stash`, `restore`, branch operations | explicit confirmation or stricter policy |

## Auto-commit strategy

```bash
akmon --auto-commit --task "Fix clippy warnings file by file and verify after each change"
```

When used correctly, this creates small auditable commits that are easier to review and revert.

## Prompt patterns that work well

```text
Summarize git diff HEAD~1 in terms of behavior changes and test risk.
```

```text
Draft a Conventional Commit message for currently staged changes.
```

```text
Compare this branch to main and list missing tests.
```

## Recommended safety flow

1. ask for analysis (`status`, `diff`),
2. apply focused edits,
3. run verification commands,
4. commit only after green checks.

## Common mistakes and troubleshooting

- **Mistake:** one huge commit for many unrelated edits.
  - **Fix:** split by concern and verify each.
- **Mistake:** running destructive git commands without review.
  - **Fix:** keep interactive approval on for mutating commands.
- **Mistake:** trusting commit message generation without diff review.
  - **Fix:** always inspect final staged diff before commit.
