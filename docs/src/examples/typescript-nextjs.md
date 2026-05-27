# Example: Next.js + TypeScript

Documented for Akmon `2.1.0`.

## Scenario

Industry/context (illustrative): fintech internal portal requiring policy-aware change traces.

## Constraints

- Approval requirement: keep server/client boundaries explicit.
- CI requirement: lint + test + E2E flow readiness.
- Audit need: headless run record for reviewer replay path.

App Router stack with **Drizzle**, **Server Actions**, and **Shadcn UI**.

## Setup

```bash
npx create-next-app@latest my-app --typescript --app --tailwind --eslint
cd my-app
npx shadcn@latest init
```

## Plan

```bash
akmon --plan --task "task management app: auth, Drizzle + Postgres,
CRUD tasks with filters, Server Components + Server Actions,
Shadcn components, middleware protection"
```

## Implement

```bash
akmon --yes --task "implement per plan; prefer RSC, use client components only where needed"
```

## Follow-ups

```
add Playwright E2E for login + task flows
```

```
add drag-and-drop ordering with optimistic UI
```

## Outcome

You get a planned and implemented Next.js slice with evidence artifacts and reproducible command history.

## Anti-patterns

- Defaulting all logic to client components without rationale.
- Shipping feature flows without test prompts in the same review cycle.
