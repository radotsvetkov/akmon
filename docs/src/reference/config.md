# Configuration reference

Documented for Akmon `3.0.0`.

## Who this is for

Operators and maintainers who need the exact supported keys in `~/.akmon/config.toml`.

## What you will have at the end

- A code-accurate list of user config keys and sections.
- Confirmed policy sections used by current CLI commands.

## Prerequisites

- Akmon installed and runnable.

## Steps

1. Resolve the active config file path.

```bash
akmon config path
```

2. Inspect current config safely.

```bash
akmon config show
akmon config show --json
```

3. Edit only supported keys listed below.

## Top-level keys (`AkmonGlobalConfig`)

```toml
default_model = "llama3.2"
ollama_url = "http://localhost:11434"

# Provider credentials (prefer env vars in CI)
# anthropic_api_key = "sk-ant-..."
# openrouter_api_key = "sk-or-..."
# openai_api_key = "sk-..."
# groq_api_key = "gsk_..."
# azure_openai_endpoint = "https://.../chat/completions"
# azure_openai_api_key = "..."
# azure_api_version = "2024-02-01"
# openai_compatible_url = "http://127.0.0.1:1234/v1"
# openai_compatible_api_key = "..."
```

## Core model keys

```toml
default_model = "llama3.2"
ollama_url = "http://localhost:11434"
```

Provider credentials can be set via env vars or config fields.

## Architect defaults (`[architect]`)

```toml
[architect]
planner_model = "llama3.2"
```

## Display settings (`[display]`)

```toml
[display]
theme = "auto" # auto | dark | light
```

## MCP servers (`[[mcp]]`)

```toml
[[mcp]]
name = "github"
url = "https://mcp.example.com"
enabled = true
scope = "user" # user | project
```

## Policy governance (`[policy]`)

```toml
[policy]
profile = "dev" # dev | staging | prod
packs = [".akmon/policy-packs/org.toml", ".akmon/policy-packs/team.toml"]
```

`profile` selects built-in defaults. `packs` adds extra policy layers.

Effective precedence:

1. selected built-in profile,
2. policy packs,
3. project-local policy (`.akmon/policy.toml` or `.akmon/policy.json`),
4. CLI override (`--policy-override`).

Within a layer, list fields append and deduplicate while keeping later precedence order.

## Policy rule schema (`PolicyConfig`)

Policy packs/local/override files use the same rule schema:

```toml
[filesystem.read]
allow = ["src/**", "Cargo.toml", "README.md"]
deny = ["src/**/secrets/**"]

[filesystem.write]
allow = ["src/**", "tests/**"]
deny = [".git/**", "**/*.pem"]

[shell]
allow_prefixes = ["cargo ", "rustfmt "]
deny_prefixes = ["cargo publish", "rm -rf "]

[network]
allow_domains = ["api.github.com", "*.rust-lang.org"]
deny_domains = ["169.254.169.254", "*.internal.local"]

[tools]
allow = ["read_*", "search", "shell"]
deny = ["shell_force", "write_secret"]

[mcp.servers]
allow = ["github-prod", "jira-main"]
deny = ["*"]

[mcp.tools]
allow = ["search_*"]
deny = ["delete_*", "admin_*"]
```

Engine behavior is deterministic:

- explicit deny beats allow,
- most specific rule wins in a rule list,
- no matching allow means deny.

For MCP actions, fail-closed behavior also applies:

- malformed/missing MCP context denies,
- ambiguous MCP context denies,
- parent policy modes without configured MCP rules deny.

## Removed sections

`[slo]`, `[slo.trend]`, and `[signing]` were removed in v3.0.0 along with `akmon slo` and `akmon sign`. If they are still in your file they are ignored, and the next `akmon config` write drops them. To sign evidence, use [`akmon seal`](./seal.md) or [`akmon bundle sign`](./bundle-sign.md).

## Model estimates (`[[model_estimates]]`)

```toml
[[model_estimates]]
pattern = "haiku-4-5"
context_window_tokens = 200000
input_per_million_usd = 1.0
output_per_million_usd = 5.0
cache_read_per_million_usd = 0.1
note = "Pricing/context hint for local estimation."
```

## Verification

```bash
akmon config show --json
akmon policy show-effective --profile dev
```

Expected result: config parses and policy can render effective configuration.

## Troubleshooting

- If `akmon config show` fails, validate TOML syntax and remove unknown keys.
- If policy packs fail to load, check file paths and TOML/JSON parse errors from `akmon policy show-effective`.
