# Python Projects

Akmon detects Python projects from **`pyproject.toml`**, **`requirements.txt`**, or **`setup.py`**.

## Auto-detection

Framework hints may include:

- **FastAPI**, **Django**, **Flask**
- **Pandas** / **Polars**
- **Scrapy**, **Celery**
- **PyTorch** / **HuggingFace**

## Conventions (steering)

Typical guidance:

- Type hints on public functions
- Pydantic v2 at API boundaries where applicable
- `pathlib` over `os.path`
- Context managers for resources
- No bare `except:`

## Example: FastAPI service

```bash
mkdir my-api && cd my-api
uv init
uv add fastapi uvicorn sqlalchemy asyncpg pydantic alembic
akmon init
akmon --plan --task "build a FastAPI service with JWT auth,
SQLAlchemy async + PostgreSQL, Alembic migrations,
Pydantic v2 schemas, APIRouter per domain"
```

## Example: Django performance

```
the order listing page is slow. analyze ORM queries
and fix N+1 issues with select_related / prefetch_related
```

## Common Python tasks

| Task | Prompt |
|---|---|
| Types | `add type hints to functions under src/` |
| Tests | `add pytest coverage for the auth module` |
| Linting | `fix ruff warnings project-wide` |
