# Introduction

Akmon is a terminal-native AI coding agent designed for developers who need control, portability, and accountability. It is intentionally built as a small Rust binary with a typed permission model, explicit provider selection, and an auditable execution trail.

As of v1.8.1, Akmon ships a complete trust pipeline:

- policy-as-code controls,
- tamper-evident audit chains,
- replay metadata and evidence artifacts,
- reliability metrics with enforceable SLO/trend gates,
- enterprise policy profiles/packs for environment rollout.

This page explains why it exists, the design choices behind it, who it is for, and where it is intentionally not trying to compete.

## The problem Akmon was built to solve

AI coding tools are now capable of shipping real features, but most teams still have the same fear: _what exactly did it do, and why did it do that?_ In many products, you get polished UX and fast output, but limited transparency into model context, permission boundaries, or reproducibility across environments. That tradeoff works for many workflows, but it breaks down for infrastructure, backend, and compliance-heavy engineering.

A second tension is provider lock-in. In practice, model quality, cost, latency, legal terms, and data handling requirements all change over time. If your coding agent is hard-wired to a single vendor, your team inherits that vendor's pricing and roadmap decisions whether or not they fit your constraints. Teams with NDAs, private source, or regulated systems often need the ability to switch providers or run local inference without changing their workflow.

A third tension is operational reality. Many developers do not work in a single desktop IDE context all day. They work over SSH in remote Linux hosts, inside ephemeral CI runners, in locked-down enterprise laptops, or in Dockerized build systems. Tools that require a specific IDE plugin stack or runtime ecosystem are often awkward or impossible in those environments. Akmon was built for those constraints first, not as an afterthought.

## The design decisions (and why)

### Single binary

Akmon is compiled Rust, shipped as a standalone executable. This is not just "nice for install." It means predictable behavior across machines because there is no dependency on a host Node/Python runtime, global package manager state, or plugin version drift. If two machines run the same Akmon version, the agent behavior is far easier to reason about.

That portability matters in practical environments:

- local development on macOS,
- remote debugging over SSH on Debian/Ubuntu,
- containerized CI jobs with minimal base images,
- controlled or air-gapped environments where runtime bootstrap is tightly restricted.

When a tool's runtime stack is small and explicit, troubleshooting is also faster. A failed run is usually a model/provider issue, permission policy issue, or repository issue, not "works on my machine because npm state differed."

### Bring your own key / bring your own model

Akmon supports Anthropic, OpenAI, OpenRouter, Groq, Azure OpenAI, Bedrock, OpenAI-compatible endpoints, and Ollama for offline local execution. You can select the model per task.

Why this matters:

- **commercial control:** you decide which provider's pricing and terms you accept,
- **privacy control:** with Ollama, code can remain local,
- **resilience:** if one provider is degraded or rate-limited, your workflow does not collapse,
- **task fit:** cheaper models for repetitive edits, stronger models for architecture/design.

In other words, model choice becomes an engineering decision, not a platform limitation.

### Typed permission system

Tool execution is not free-form shell by default. Operations pass through permission checks modeled as typed actions (read file, write file, execute command, network fetch, and so on). The user can approve once, approve for a session, or deny based on policy and context.

That creates a concrete safety boundary between model suggestions and actual side effects. The model can _request_ actions; it cannot silently mutate the system without policy passing those requests.

### Audit log

Akmon can emit JSONL audit events with session metadata, policy decisions, tool calls, and execution flow. This is essential for teams that need post-run review and accountability.

Practically, it gives you answers to questions like:

- what command executed and when,
- what file was modified and after which approval,
- where the session stopped,
- what model and token usage were involved.

The point is not surveillance; the point is operational clarity when AI performs meaningful work on production code.

### Context as architecture

Akmon's workflow emphasizes context discipline: research, plan/spec, then implementation. That structure exists because LLM performance degrades when context fills with stale exploration artifacts.

Instead of trying to keep one giant conversation forever, Akmon encourages:

1. focused exploration,
2. explicit plan/spec on disk (`.akmon/specs`),
3. implementation from plan with iterative verification.

This makes sessions more predictable, easier to resume, and less likely to drift into repetitive read loops.

## Who Akmon is for

### Backend developers working across varied environments

If you work in terminal-first repositories, remote hosts, and client-controlled systems where installing editor plugins is inconsistent, Akmon's single-binary model is a practical fit.

### Platform and DevOps engineers automating changes

Headless mode (`--task`, `--yes`, `--output json`) is designed for automation and CI integration. Budget controls and structured outputs make it easier to gate and observe autonomous runs.

### Regulated or privacy-constrained teams

Teams that need to constrain data movement can run local models with Ollama and keep a durable audit record for operational/compliance review.

### Open-source maintainers avoiding vendor coupling

Akmon is Apache-2.0 and provider-agnostic. You can evolve model strategy over time without replacing the tool itself.

## What Akmon does not do

Akmon is intentionally opinionated, and that includes explicit limits:

- It is not an IDE extension with inline completions and graphical diff UX.
- It does not try to out-polish provider-native tools in their own primary environment.
- Local models can be slower and less capable on complex multi-file design tasks.
- It is terminal-first and expected to remain that way.

Those are deliberate tradeoffs in favor of portability, policy boundaries, and scriptability.

## Common mistakes and troubleshooting

### Mistake: using one expensive model for every task

Use a cheaper model for exploration/refactors and reserve stronger models for architecture-heavy reasoning.

### Mistake: skipping project context

Create and maintain `AKMON.md`. A short, high-quality project context usually improves outputs more than writing longer prompts each turn.

### Mistake: running headless without budget limits

For CI and automation, use `--max-budget-usd` and parse `--output json` for safe exits.

### Mistake: blaming model quality for policy friction

If the agent seems "stuck," inspect permission prompts and policy settings first. In many cases the model is waiting for explicit approval.

## Where to go next

- Install and first run: [Getting Started](./getting-started/installation.md)
- Practical walkthroughs: [Step-by-step tutorials](./tutorials/step-by-step.md)
- Automation and CI: [Headless mode](./usage/headless.md)
- Integration with external systems: [MCP guide](./features/mcp.md)
- Internal architecture for contributors: [Contributing architecture](./contributing/architecture.md)
