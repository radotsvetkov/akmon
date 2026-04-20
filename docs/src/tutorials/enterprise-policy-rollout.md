# Tutorial C: Enterprise policy profile rollout

Roll policy governance from `dev` to `staging` to `prod` with deterministic merge behavior.

## 1) Start with `dev`

```bash
akmon policy show-effective --profile dev
akmon chat --policy-profile dev
```

`dev` is read-friendly with controlled mutation defaults.

## 2) Add an organizational policy pack

Create `.akmon/policy-packs/org.toml`:

```toml
[tools]
deny = ["shell"]

[network]
deny_domains = ["*"]
```

Apply and inspect:

```bash
akmon policy show-effective --profile dev --policy-pack .akmon/policy-packs/org.toml --output json
```

## 3) Move to `staging`

```bash
akmon policy show-effective --profile staging --policy-pack .akmon/policy-packs/org.toml
akmon --policy-profile staging --policy-pack .akmon/policy-packs/org.toml --task "run non-mutating checks"
```

## 4) Lock down with `prod`

```bash
akmon policy show-effective --profile prod --policy-pack .akmon/policy-packs/org.toml
```

Use `prod` for high-assurance automation where explicit-deny posture is required.

## 5) Allow/deny behavior checks

```bash
# Expect deny (prod + pack)
akmon --policy-profile prod --policy-pack .akmon/policy-packs/org.toml --task "run shell command: cargo test"

# Expect allow for read-heavy tasks
akmon --policy-profile prod --task "list auth module files and summarize"
```

Merge precedence reminder:

`profile < packs < project-local policy < CLI override`
