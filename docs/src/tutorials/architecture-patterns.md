# Architecture patterns for agent-assisted development

Akmon does not dictate your system architecture—it **documents and executes** against the architecture you describe. These patterns work well in practice.

## Single repo, single source of truth (`AKMON.md`)

**When:** Small teams, one deployable.

**How:**

- Maintain **Product**, **Architecture**, **Conventions**, **Current sprint** in `AKMON.md` ([reference](../project/akmon-md.md)).
- Run `akmon --plan` before refactors that touch many modules.
- Use `/update-context` in the TUI when editing `AKMON.md` mid-session.

**Risk:** Drift if nobody updates **Current sprint**—treat it like a stand-up note.

## Planner / implementer split

**When:** Expensive frontier models, or you want cheap exploration before precise edits.

**How:**

- [Architect mode](../usage/architect-mode.md): planner model writes a plan file, main model implements.
- Or manual: `--plan` + review + `--yes --task`.

**Risk:** Plans can go stale; re-run plan if main branch moved.

## Spec-driven (requirements → design → tasks)

**When:** New features with unclear scope; need written artifacts for review.

**How:** [Spec workflow](../usage/spec-workflow.md) — `akmon spec …` phases under `.akmon/specs/`.

**Risk:** More ceremony; best when the spec is genuinely reviewed by humans.

## Automation + human gate

**When:** Nightly hygiene (formatters, comment sweeps) without full write access in CI.

**How:**

- Narrow `--task` in automation; require PR for merges.
- Keep destructive operations out of unattended jobs.

**Risk:** Broad tasks in `--yes` can still surprise you—scope tightly.

## Multi-service repos (monorepo)

**When:** Several services share conventions.

**How:**

- One `AKMON.md` at root with **Architecture** mapping services; or per-service nested guides linked from root.
- Run Akmon from the **service subdirectory** when the sandbox should be minimal.

**Risk:** Wrong working directory → wrong paths; always `cd` to the intended git root.

## Documentation as contract

**When:** Regulated or long-lived systems.

**How:**

- Commit plans under `.akmon/plans/` as part of the change record.
- Export audit JSONL to your retention system.

**Risk:** Treat plans as non-authoritative unless your process says otherwise—they are agent output, not legal sign-off.

## Choosing a pattern

| Need | Start with |
| --- | --- |
| Fast iteration | Interactive `akmon chat` + tight **Current sprint** |
| Cost control | Plan mode + local model, then targeted `--yes` |
| Large redesign | Architect mode or explicit `--plan` review |
| New product area | Spec workflow |
| CI | Headless JSON + narrow tasks + read-heavy defaults |
