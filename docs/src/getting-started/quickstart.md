# Quick Start

Documented for Akmon `2.1.0`.

## Who this is for

Engineers who want a first successful Akmon run in a local repository using either a local model (Ollama) or hosted provider credentials.

## What you will have at the end

- One completed Akmon session in your project.
- A recorded audit log and evidence artifact for that session.
- A baseline understanding of interactive and headless entry points.

## Prerequisites

1. `akmon --version` works.
2. You are inside a git repository.
3. One provider path is ready:
   - Local: `ollama` running with a pulled model.
   - Hosted: one of `ANTHROPIC_API_KEY`, `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `GROQ_API_KEY`, or Azure/Bedrock settings.

## Steps

1. Choose a model source.

```bash
# Local-first example
ollama pull qwen2.5-coder:7b
```

```bash
# Hosted example (pick one)
export ANTHROPIC_API_KEY="YOUR_KEY"
```

Expected result: provider prerequisites are available before Akmon starts.

2. Start Akmon in interactive mode.

```bash
cd /path/to/your-repo
akmon
```

Expected result: full-screen TUI opens for a new session.

3. Run a read-only exploration prompt.

```text
explain where authentication state is created and validated
```

Expected result: Akmon reads/searches files and returns an explanation with tool traces in-session.

4. Run one bounded implementation prompt.

```text
add input validation to create_user and explain the tests required
```

Expected result: Akmon proposes file edits with approval gates before writes.

5. End the session cleanly.

```text
/exit
```

Expected result: session summary appears and Akmon saves artifacts.

## Verification

Check that session evidence exists in the project:

```bash
ls -1 .akmon/audit .akmon/evidence
```

Then verify integrity:

```bash
# Replace with your session UUID
akmon verify <session-uuid>
```

Expected result:
- `akmon verify` exits `0` for a valid session chain.
- Audit/evidence files are present for reviewer handoff.

## Troubleshooting

- If startup fails with provider errors, run `akmon doctor providers`.
- If model routing is unclear, run `akmon config explain-provider`.
- If Ollama is slow on first turn, warm model once with `ollama run <model>`.
- If you need machine-readable output for CI, use headless mode:

```bash
akmon --task "run tests and summarize failures" --output json --yes
```
