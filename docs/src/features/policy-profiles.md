# Policy profiles and packs

Documented for Akmon `2.2.0`.

Policy profiles and packs govern Akmon's own reference agent. They decide what side effects the bundled agent may take and how strictly each environment is locked down. This is a producer-side control: it shapes the behavior of a reference-agent run, and the effective policy is hashed into the run's evidence so a reviewer can detect governance drift. It does not apply to imported third-party OpenTelemetry traces, which carry only what the producing agent emitted.

Akmon supports enterprise policy rollout with reusable profiles and composable packs, so the same governance inputs can move from a developer laptop to a hardened CI runner without rewriting rules.

## Built-in profiles

- `dev`: read-friendly, controlled writes, restricted shell and network.
- `staging`: stricter write, shell, and network posture than `dev`.
- `prod`: highly restrictive, explicit-deny posture for side effects.

Profiles map to the existing `PolicyConfig` schema (filesystem, shell, network, tools).

## Policy packs

Policy packs are local TOML or JSON policy files layered on top of a selected profile.

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

1. built-in profile,
2. packs,
3. project-local policy (`.akmon/policy.toml` or `.akmon/policy.json`),
4. CLI override (`--policy-override`).

Within each layer, list fields append and deduplicate while keeping the last occurrence, so higher-precedence layers keep later rule order. Evaluation within a rule list is deterministic: explicit deny wins, and the most specific matching rule is selected.

## Inspect effective policy

Use:

```bash
akmon policy show-effective --profile staging --policy-pack .akmon/policy-packs/org.toml
akmon --output json policy show-effective --profile prod
```

This prints the final merged policy and the exact source order used.

## Governance provenance in evidence

The effective policy after the merge is hashed into the run's evidence as `replay_metadata.policy_hash`. Because the hash is deterministic, any change to the selected profile or to pack contents changes the hash. A CI or PR system can therefore detect a policy-governance change between runs even when the behavioral effect is subtle. This is what makes the policy layer auditable rather than merely enforced. See [Evidence artifact](./evidence.md).

## Rollout guidance

Typical enterprise rollout:

1. Start with `dev` plus narrow team packs.
2. Tighten shell, network, and tool scope in `staging`.
3. Lock production automation to `prod` plus an audited, minimal override pack.
4. Enforce evidence and SLO checks in CI after policy changes, and gate on `policy_hash` to catch unreviewed governance drift.

For a step-by-step rollout, see the [Enterprise policy rollout tutorial](../tutorials/enterprise-policy-rollout.md).

## See also

- [Security model](./security.md)
- [Evidence artifact](./evidence.md)
- [Reliability and SLO metrics](./reliability-slos.md)
