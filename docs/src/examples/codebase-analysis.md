# Example: Analyzing a codebase

Documented for Akmon `2.1.0`.

## Scenario

Industry/context (illustrative): regulated fintech service onboarding a new engineer for read-only architecture review.

## Constraints

- Data boundary: no uncontrolled writes during discovery.
- Approval requirement: keep first pass read-oriented.
- Audit need: produce artifacts reviewers can verify.

## Akmon workflow

```bash
cd unfamiliar-project
akmon init
akmon --plan --task "map architecture, auth path, config flow, and top risk modules"
akmon --yes --output json --task "summarize architecture and list highest-risk modules with evidence links" | tee analysis-run.json
```

## Useful prompt patterns

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

## Outcome

After the run, you should have:
- `analysis-run.json`
- `.akmon/audit/<session-id>.jsonl`
- `.akmon/evidence/<session-id>.json`

Reviewer question this answers: "What did the analyst inspect, and is the summary traceable to a verifiable session?"

## Anti-patterns

- Using broad write tasks during initial architecture discovery.
- Reporting "risk areas" without keeping the corresponding session evidence.
