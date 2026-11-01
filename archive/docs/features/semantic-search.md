# Semantic search

Semantic search lets Akmon find relevant code by meaning, not only exact keyword matches.

## When to use it

Use semantic search for questions like:

- "where do we validate JWTs?",
- "what code handles retry/backoff?",
- "where is this business rule enforced?"

It is especially useful when symbols are named inconsistently across a large codebase.

## Enabling semantic search

Run Akmon with indexing enabled:

```bash
akmon chat --index
```

On first run, the index build may take time depending on repository size.

## Practical workflow

1. ask a high-level question,
2. review candidate files from semantic results,
3. use exact text search/read tools to verify before editing.

Semantic search should guide exploration, not replace source validation.

## Cost and context implications

Semantic search can reduce wasted context by narrowing file reads to likely matches instead of broad brute-force scans.

Best practice:

- use semantic search for discovery,
- follow with targeted file reads and scoped edits.

## Common mistakes and troubleshooting

- **Mistake:** treating semantic results as ground truth.
  - **Fix:** always confirm by reading source files.
- **Mistake:** expecting semantic indexing in slim builds.
  - **Fix:** verify your build/runtime mode and `--index` usage.
- **Mistake:** indexing generated/vendor directories.
  - **Fix:** ensure ignore files exclude noisy paths.

See also [CLI reference](../reference/cli.md) and [Capabilities](../reference/capabilities.md).
