# Example: Refactoring an existing project

Documented for Akmon `2.1.0`.

## Scenario

Industry/context (illustrative): medical software backend needs controlled refactoring with explicit reviewer traceability.

## Constraints

- Approval requirement: no silent architecture rewrites.
- CI requirement: tests must pass after each scoped change.
- Audit need: maintain session evidence for each refactor batch.

## Prepare

```bash
cd existing-project
akmon init
```

Edit **`AKMON.md`** so conventions match your team before large refactors.

## Akmon workflow patterns

**Extract service layer**

```
handlers still contain DB + business rules.
move rules to src/services and persistence to src/repositories
```

**Harden error handling**

```
replace unwrap/expect with proper error types and propagation
```

**Add tests**

```
add tests for src/auth — happy paths, invalid input, mocked I/O
```

**Performance**

```
profile hot paths; propose top 3 bottlenecks with fixes
```

**Security audit**

```
review API handlers: validation, authz gaps, sensitive data in logs
```

Akmon shows diffs before writes — review each change.

## Outcome

You should produce:
- Small, reviewable refactor batches.
- Evidence artifacts per batch run.
- A reviewer-auditable path from prompt to file changes.

## Anti-patterns

- Asking for "full project refactor" in one turn.
- Skipping verification/tests between structural changes.
