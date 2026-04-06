# Git Integration

Git operations are exposed as **tools** with policy + approval semantics.

## Typical operations

| Area | Examples |
| --- | --- |
| Read-only | `status`, `diff`, `log`, `show` |
| Writes | `add`, `commit`, `stash`, `restore`, … |

Read-only operations are easier to auto-approve under `--yes`; mutating commands require explicit consent.

## Auto-commit mode

```bash
akmon --auto-commit --task "fix clippy issues file by file"
```

Each approved write may become its **own commit**, simplifying review and `git revert`.

## Prompts that pair well with git

```
summarize git diff HEAD~1
```

```
draft a Conventional Commit message for staged changes
```

```
compare this branch to main — risks and test gaps
```

Git context helps Akmon reason about **what changed** and **what to test next**.
