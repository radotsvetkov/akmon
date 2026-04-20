# Policy Profiles & Packs

Akmon supports enterprise policy rollout with reusable profiles and composable packs.

## Built-in profiles

- `dev`: read-friendly, controlled writes, restricted shell/network
- `staging`: stricter write/shell/network posture than `dev`
- `prod`: highly restrictive, explicit-deny posture for side effects

Profiles map to the existing `PolicyConfig` schema (filesystem, shell, network, tools).

## Policy packs

Policy packs are local TOML/JSON policy files layered on top of a selected profile.

Default discovery path:

```text
.akmon/policy-packs/*.toml
.akmon/policy-packs/*.json
```

Additional packs can be added with repeatable CLI flags:

```bash
akmon --policy-pack .akmon/policy-packs/org.toml --policy-pack .akmon/policy-packs/team.toml --task "..."
```

Malformed selected packs fail closed with an explicit error.

## Deterministic precedence

Effective policy merge order:

1. built-in profile
2. packs
3. project-local policy (`.akmon/policy.toml` or `.akmon/policy.json`)
4. CLI override (`--policy-override`)

Within each layer, list fields append and deduplicate while keeping the last occurrence, so higher-precedence layers keep later rule order.

## Inspect effective policy

Use:

```bash
akmon policy show-effective --profile staging --policy-pack .akmon/policy-packs/org.toml
akmon --output json policy show-effective --profile prod
```

This prints the final merged policy and the exact source order used.

## Rollout guidance

Typical enterprise rollout:

1. Start with `dev` + narrow team packs.
2. Tighten shell/network/tool scope in `staging`.
3. Lock production automation to `prod` + audited minimal override pack.
4. Enforce evidence/SLO checks in CI after policy changes.
