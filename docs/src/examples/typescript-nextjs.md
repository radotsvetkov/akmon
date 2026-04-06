# Example: Next.js + TypeScript

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
