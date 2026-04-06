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
    SUMMARY.md       Table of contents — defines navigation
    introduction.md  Landing page
    getting-started/ Installation, quickstart, providers
    usage/           Interactive, headless, plan, spec modes
    project/         init, AKMON.md, import, export
    languages/       Rust, Python, TypeScript, Go guides
    examples/        Complete worked examples
    features/        Audit log, security, cost, git, MCP
    providers/       Per-provider setup guides
    reference/       CLI, slash commands, env vars
    contributing/    Setup, architecture, adding providers
  theme/
    custom.css       Amber accent, tip/warning callouts
```
