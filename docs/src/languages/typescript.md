# TypeScript Projects

Akmon detects TypeScript from **`tsconfig.json`** and frameworks from **`package.json`** dependencies.

## Auto-detection

- **Next.js** (App Router patterns)
- **React**
- **NestJS**
- **Prisma**, **Drizzle**, **tRPC**, **Hono**

## Conventions (steering)

- `strict: true`
- Avoid `any` — prefer `unknown` + narrowing
- **Zod** (or similar) at API boundaries
- Path aliases instead of deep relative imports
- Discriminated unions for state machines

## Example: Next.js

```bash
npx create-next-app@latest my-app --typescript --app
cd my-app
akmon init
```

```
add authentication with your chosen stack (e.g. auth library + DB)
using Server Actions where appropriate
```

## Common TypeScript tasks

| Task | Prompt |
|---|---|
| Types | `replace any with proper types` |
| Validation | `add Zod schemas to API handlers` |
| Tests | `add Vitest tests for auth helpers` |
