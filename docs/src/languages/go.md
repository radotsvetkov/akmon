# Go Projects

Akmon detects Go projects from **`go.mod`** and frameworks from module requirements.

## Auto-detection

- **Gin**, **Echo**, **Chi**, **Fiber**
- **Cobra** CLIs
- **GORM**, **sqlc**, **ent**
- **`errgroup`** patterns

## Conventions (steering)

- Check every error; never silently discard with `_`
- Accept interfaces, return concrete types
- `context.Context` first on I/O boundaries
- Table-driven tests with `t.Run`

## Example: Gin API

```bash
mkdir my-api && cd my-api
go mod init example.com/my-api
go get github.com/gin-gonic/gin gorm.io/gorm gorm.io/driver/postgres
akmon init
akmon --plan --task "REST API for a blog with Gin, GORM + Postgres,
JWT middleware, handler → service → repository layout"
```

## Common Go tasks

| Task | Prompt |
|---|---|
| Errors | `find ignored errors and handle them` |
| Context | `thread context through service methods` |
| Tests | `add table-driven tests for package X` |
