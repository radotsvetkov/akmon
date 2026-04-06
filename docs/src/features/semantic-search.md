# Semantic search

Akmon can optionally use **semantic (embedding) search** over your codebase when built with the semantic index feature and started with **`--index`**.

## Enable

Build the full binary (with indexing dependencies), then:

```bash
akmon chat --index
```

A binary index is stored under `.akmon/` (for example `.akmon/index.bin`). The first run may take time to embed files.

## What it does

- **`semantic_search` tool** — finds code or docs by meaning, not only exact text.
- Complements **text search** (`grep`-style) for exploration and large repos.

## Without `--index`

The slim build or runs without `--index` omit the semantic search tool; **read**, **search**, and **list** tools still work.

## Tips

- Regenerate or refresh index when you change many files (behavior depends on version; check CLI help).
- Add huge generated dirs to `.gitignore`; indexing usually respects project boundaries.

See [CLI reference](../reference/cli.md) for flags and [Development setup](../contributing/setup.md) for full vs slim builds.
