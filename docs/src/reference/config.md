# Configuration reference

Akmon user config is stored at `~/.akmon/config.toml`.

## Core model keys

```toml
default_model = "llama3.2"
ollama_url = "http://localhost:11434"
```

Provider credentials can be set via env vars or `akmon config key`.

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
```

Engine behavior is deterministic:

- explicit deny beats allow,
- most specific rule wins in a rule list,
- no matching allow means deny.

## Reliability defaults (`[slo]`)

```toml
[slo]
min_tool_success_rate = 0.95
max_timeout_rate = 0.02
max_policy_denial_rate = 0.20
max_tool_failure_rate = 0.05
max_retries_total = 3
max_timeouts_total = 2
min_tool_calls_total = 5

[slo.trend]
max_success_rate_drop_abs = 0.05
max_timeout_rate_increase_abs = 0.02
max_failure_rate_increase_abs = 0.03
max_retries_increase_ratio = 1.0
max_latency_avg_increase_ratio = 0.50
min_baseline_samples = 5
```

CLI overrides take precedence over config.

## Migration notes for v1.8.0 operators

- Audit records are chain-shaped (`schema_version`, `event_index`, `prev_hash`, `event_hash`).
- Run report JSON now includes additive `replay_metadata` and `reliability_metrics`.
- Evidence artifacts are versioned (`evidence_schema_version: "evidence.v1"`).
- Policy governance can now be managed by profile/packs without changing permission classes.
