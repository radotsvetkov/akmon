# Example: CLI tool with Python

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
