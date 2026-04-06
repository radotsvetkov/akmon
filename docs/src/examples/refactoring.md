# Example: Refactoring an existing project

## Prepare

```bash
cd existing-project
akmon init
```

Edit **`AKMON.md`** so conventions match your team before large refactors.

## Patterns

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
