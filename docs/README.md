# Akmon Documentation

Built with [mdBook](https://rust-lang.github.io/mdBook/).

Live at: https://radotsvetkov.github.io/akmon

## Local development

```bash
cargo install mdbook
cd docs
mdbook serve
# Open http://localhost:3000
```

## Docs quality checks

Run the same docs checks CI runs:

```bash
bash scripts/docs/run_all.sh
```

This includes mdBook build, link checks, CLI smoke checks, JSON snippet checks, and version-marker checks (every `Documented for Akmon X.Y.Z` marker must match the workspace version).

## Maintainers workflow

See `docs/MAINTAINERS.md` for the docs update protocol, quarterly audit checklist, and tutorial definition of done.

## Adding a page

1. Create a `.md` file in the appropriate `docs/src/` subdirectory
2. Add it to `docs/src/SUMMARY.md` in the right location
3. Write content in standard Markdown
4. Push to main — the site rebuilds automatically

## Structure

```
docs/
  book.toml          mdBook configuration
  src/
    SUMMARY.md       Table of contents: defines navigation
    introduction.md  Landing page: the one job (seal, hand over, verify offline)
    getting-started/ Quick start, install, and reference-agent provider/config setup
    concepts/        Trust model, capture levels, what verification proves, compliance
    guides/          Seal an OTEL trace, hand evidence over, timestamps, retention
    use-cases/       Air-gapped verification, operator sign-off, release evidence pack
    reference/       One page per command (seal, otel import, bundle *, agef-verify, ...)
    usage/           Bundled reference agent: interactive, headless, plan, architect
    features/        Bundled reference agent: policy, security, MCP, git, cost, legacy logs
    providers/       Per-provider setup for the reference agent
    languages/       Language guides for the reference agent
    tutorials/       Longer walkthroughs
    releases/        Release notes (historical pages keep their own versions)
    contributing/    Setup, architecture, adding providers
  theme/
    custom.css       Accent styling
```

Pages for features removed in v3.0.0 live under `archive/docs/` and are not built.
