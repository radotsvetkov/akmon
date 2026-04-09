# MCP integration guide

MCP (Model Context Protocol) lets Akmon call external tools on demand: databases, issue trackers, internal APIs, docs systems, and more.

## What MCP gives you

Without MCP, developers often paste large external context (schema dumps, issue text, docs) directly into prompts. That is expensive and fragile. MCP changes this model: the agent requests only the data it needs, when it needs it.

Benefits:

- keeps large datasets out of the core prompt context,
- improves context-window efficiency,
- reduces manual copy/paste operations,
- allows repeatable integrations across projects.

## Setting up MCP servers

Example configuration in `~/.akmon/config.toml`:

```toml
[[mcp_servers]]
name = "postgres"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres", "postgresql://localhost/myapp"]

[[mcp_servers]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_..." }

[[mcp_servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/you/documents"]
```

Then inspect:

```bash
akmon config mcp list
akmon config mcp test postgres
akmon config mcp test github
```

## Real workflow: database-driven development

Scenario: you need SQLAlchemy models from a live schema and do not want to paste DDL.

```bash
akmon --model claude-haiku-4-5-20251001
```

Prompt:

```text
Use available MCP tools to inspect the PostgreSQL schema, then generate SQLAlchemy models and CRUD routes that match current tables and relationships.
```

Expected behavior:

1. agent discovers MCP tools for postgres,
2. queries schema metadata via MCP tool calls,
3. writes model files from real schema,
4. verifies via project test/lint commands.

## Real workflow: GitHub issue execution

Prompt:

```text
Use GitHub MCP to read issue #47, implement the requested change, and create a commit message referencing the issue number.
```

Expected behavior:

- reads issue content directly from GitHub,
- finds local files to change,
- produces implementation + verification commands,
- prepares commit summary linked to issue context.

## Real workflow: external filesystem context

Prompt:

```text
Read ~/documents/api-spec.md via filesystem MCP and update this repository's API handlers to match it.
```

This is useful when specs, contracts, or governance docs live outside the repository root.

## Building a custom MCP server (minimal Python example)

```python
#!/usr/bin/env python3
import json
import sys

TOOLS = [{"name": "hello_company", "description": "Returns internal greeting"}]

for line in sys.stdin:
    req = json.loads(line)
    method = req.get("method")
    rid = req.get("id")
    if method == "tools/list":
        print(json.dumps({"id": rid, "result": {"tools": TOOLS}}), flush=True)
    elif method == "tools/call":
        name = req.get("params", {}).get("name")
        if name == "hello_company":
            print(json.dumps({"id": rid, "result": {"content": "hello from internal system"}}), flush=True)
        else:
            print(json.dumps({"id": rid, "error": {"message": "unknown tool"}}), flush=True)
```

Wire this script as an MCP server command in config.

## Safety and policy model

MCP is not a bypass:

- calls still pass through Akmon policy checks,
- potentially destructive actions can still require confirmation,
- audit logs still record actions and outcomes.

Treat MCP servers like production dependencies: least privilege, scoped credentials, and explicit ownership.

## Common mistakes and troubleshooting

- **Server starts locally but `mcp test` fails:** check command path and env vars.
- **Tool missing in session:** verify server is enabled and reachable from runtime shell.
- **Slow responses:** reduce response size in MCP server output; return focused payloads.
- **Risky action exposure:** split read-only and write-capable tools into separate servers/credentials.
