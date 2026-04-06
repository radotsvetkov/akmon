# Example: Analyzing a codebase

## Bootstrap context

```bash
cd unfamiliar-project
akmon init
akmon chat
```

## Questions that work well

```
high-level architecture and main data flows
```

```
trace the auth path from HTTP request to response
```

```
list the riskiest modules if they were wrong
```

```
where is configuration loaded and how does it propagate?
```

```
find external HTTP calls missing timeouts
```

```
generate a Mermaid diagram of major modules
```

```
which tests should I run locally before contributing?
```

Use **`/plan`** for read-only reconnaissance on sensitive trees.
