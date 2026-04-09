# Other tools vs Akmon

This page is intentionally short. Products change quickly—always check vendor docs for the latest.

## The idea

**Typical coding agents** (editor extensions, hosted terminals, bundled stacks) optimize for speed and integration inside a vendor’s world—subscriptions, credits, and their chosen runtime.

**Akmon** optimizes for **control**: one binary you own, **bring-your-own model**, an **explicit permission layer**, and a **JSONL audit log** of what happened in each session.

Neither is “better” everywhere—it depends whether you care more about seamless IDE glue or about a small, inspectable tool you can run in CI, over SSH, or air‑gapped with Ollama.

## Side-by-side (rough)

| | Typical agents | Akmon |
| --- | --- | --- |
| **Shape** | Often an app + runtime (e.g. Node), or IDE‑bound | Single static binary (Rust), optional features at compile time |
| **Subscription** | Common for hosted products | No subscription for the agent; you pay APIs you choose |
| **Audit trail** | Varies; rarely a full per-session JSONL of policy + tools | Designed around JSONL audit events |
| **Models** | Often tied to one vendor or plan | Ollama locally; Anthropic, OpenAI‑compat, OpenRouter, Bedrock, etc. |
| **Trust boundaries** | Varies | Repo sandbox, typed permissions, SSRF‑aware optional `web_fetch` |

## Examples people compare us to

Names like **Claude Code**, **Cursor**, **Cline**, and **Aider** solve overlapping problems with different tradeoffs (IDE vs terminal, bundled vs BYOK, multi-file UX, etc.). Use what fits your workflow; use Akmon when you want the **forge** to be **yours**.

---

[← Introduction](./introduction.md) · [Security model →](./features/security.md)
