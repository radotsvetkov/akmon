# Example: CLI tool with Python

Documented for Akmon `2.0.0`.

## Scenario

Industry/context (illustrative): regulated ops tooling where command outputs feed compliance reports.

## Constraints

- Data boundary: avoid leaking secrets in CLI output/logs.
- Approval requirement: explicit schema validation for external API fields.
- CI requirement: stable command behavior with tests.

**Typer + Rich + Pydantic** is a solid stack for user-friendly CLIs.

## Setup

```bash
mkdir my-cli && cd my-cli
uv init
uv add typer rich pydantic httpx
```

## Plan

```bash
akmon --plan --task "CLI for GitHub repo stats: stats OWNER/REPO,
compare REPO1 REPO2, trending [lang]. Typer + Rich tables,
Pydantic models for API JSON, httpx async client,
config file under ~/.config/my-cli/"
```

## Implement

```bash
akmon --yes --task "implement the CLI per the saved plan"
```

## Follow-ups

```
add JSON output mode for scripting
```

```
add shell completions via Typer
```

## Outcome

You get a structured CLI implementation path plus traceable Akmon run artifacts for review.

## Anti-patterns

- Treating API JSON as untyped dicts in production-facing commands.
- Merging CLI UX and transport refactors into one unreviewed session.
