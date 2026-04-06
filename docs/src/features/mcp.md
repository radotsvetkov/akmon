# MCP Tools

Akmon can load **Model Context Protocol** servers so the agent may call external tools (issue trackers, databases, internal APIs) with the same permission posture as first-party tools.

## Configure

Via CLI (see `akmon config mcp --help` for your version) or `~/.akmon/config.toml`:

```toml
[[mcp.servers]]
name = "github"
url = "https://example.com/mcp"
enabled = true
```

## Manage

```bash
akmon config mcp list
akmon config mcp test <name>
akmon config mcp remove <name>
```

## TUI

```
/mcp
```

Shows connection health and discovered tools when supported.

## Safety

MCP calls are still subject to **policy**, **confirmation**, and **audit logging** — they are not a silent backdoor.

## Example prompts

```
list open issues labeled bug for this repo
```

```
run a read-only SQL diagnostic (if your MCP exposes it safely)
```

If a server can perform destructive actions, treat it like production access: least privilege, network controls, and code review.
