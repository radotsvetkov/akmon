# Example projects

Copy-paste starter flows for common stacks.

## Rust API starter

```bash
mkdir -p ~/projects/rust-api && cd ~/projects/rust-api
git init
akmon --yes --output json --task "create an Axum REST API skeleton with tests" | tee run.json
```

Expected artifacts:

- `.akmon/audit/<session-id>.jsonl`
- `.akmon/evidence/<session-id>.json`
- run report `run.json`

## Python FastAPI starter

```bash
mkdir -p ~/projects/fastapi-service && cd ~/projects/fastapi-service
git init
akmon --yes --output json --task "create FastAPI app with /health endpoint and pytest test" | tee run.json
```

Expected artifacts:

- `.akmon/audit/<session-id>.jsonl`
- `.akmon/evidence/<session-id>.json`
- run report `run.json`

## Node service starter

```bash
mkdir -p ~/projects/node-service && cd ~/projects/node-service
git init
akmon --yes --output json --task "create Node TypeScript service with basic route and test" | tee run.json
```

Expected artifacts:

- `.akmon/audit/<session-id>.jsonl`
- `.akmon/evidence/<session-id>.json`
- run report `run.json`
