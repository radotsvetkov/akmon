# Elixir / Phoenix — context-first agent sessions

Documented for Akmon `2.0.0`.

## Scenario

Industry/context (illustrative): defense supplier internal portal with strict module ownership and review controls.

## Constraints

- Approval requirement: context/module boundaries must remain explicit.
- CI requirement: `mix test` required after implementation.
- Audit need: preserve evidence for each feature-slice run.

Beam projects reward **explicit module names** and **Mix task** discipline. Akmon does not run `mix` for you unless you approve shell tool calls—so naming conventions in `AKMON.md` matter.

## Prepare `AKMON.md`

Include:

- **Umbrella or single app** layout (`apps/` vs `lib/`).
- **Contexts** that own domain logic (e.g. `MyApp.Accounts`).
- Preferred checks: `mix test`, `mix format`, optional `mix dialyzer`.

## Feature slice — LiveView or controller

```bash
akmon --plan --task "Plan a settings page restricted to admin users; list schemas, contexts, and tests to add"
```

Review `.akmon/plans/` then:

```bash
akmon --yes --task "Implement the admin settings page per plan; add ExUnit tests for context functions"
mix test
```

## Why plan mode helps

Phoenix moves through **router → controller/live → context → schema**. A plan file forces the agent to state that order before editing, which reduces half-written plugs or misnamed `assigns`.

## Multi-agent note

You can run a local model for planning and a hosted model for implementation on the same repository when policy allows.

See [Other languages](../languages/other.md) for Elixir mentions in the generic profile path.

## Outcome

The result is a context-aligned Phoenix feature slice with a reviewable plan-to-implementation trace.

## Anti-patterns

- Editing router, contexts, and schemas without an explicit plan artifact first.
- Treating Mix checks as optional in regulated delivery workflows.
