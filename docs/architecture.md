# Architecture

This file used to describe the v1.x crate layout. The current architecture is documented in the handbook:

- [How Akmon works](src/concepts/architecture.md): the evidence model (journal, AGEF bundle, signatures, attestations, verification) and the 13-crate layout.
- [Architecture guide for contributors](src/contributing/architecture.md): the bundled reference agent's internals (session loop, tools, TUI).

In short: the evidence path (`akmon-journal`, `akmon-bundle`, `agef-verify`, `akmon-otel`) is independent of the reference-agent crates (`akmon-query`, `akmon-models`, `akmon-tools`, `akmon-tui`). `akmon-replay` and `akmon-diff` work on journal sessions, and `akmon-cli` wires everything into the `akmon` binary.
