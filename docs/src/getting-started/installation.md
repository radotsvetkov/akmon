# Installation

## Requirements

- **Git** — for project context and git operations
- **Rust 1.88+** — only if building from source
- **Ollama** — optional, for local offline models

## Option 1 — Pre-built binary (recommended)

**macOS — Apple Silicon**
```bash
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-darwin-arm64 \
  -o /usr/local/bin/akmon && chmod +x /usr/local/bin/akmon
```

**macOS — Intel**
```bash
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-darwin-x86_64 \
  -o /usr/local/bin/akmon && chmod +x /usr/local/bin/akmon
```

**Linux — x86_64**
```bash
curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 \
  -o /usr/local/bin/akmon && chmod +x /usr/local/bin/akmon
```

**Verify**
```bash
akmon --version
# e.g. akmon 1.5.1
```

## Option 2 — From source

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon

# Slim build — no semantic indexing, smaller binary
cargo build --release --no-default-features

# Full build — with semantic indexing
cargo build --release

cp target/release/akmon /usr/local/bin/
```

## Option 3 — cargo install

```bash
cargo install akmon
```

## Using over SSH

Akmon is a single static binary. Copy it to any remote machine:

```bash
scp /usr/local/bin/akmon user@remote:/usr/local/bin/
ssh user@remote
akmon chat
```

## Using in Docker

```dockerfile
FROM debian:bookworm-slim
RUN curl -L \
  https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 \
  -o /usr/local/bin/akmon && chmod +x /usr/local/bin/akmon
WORKDIR /workspace
ENTRYPOINT ["akmon"]
```

## Using in CI

```yaml
# GitHub Actions example
- name: Install Akmon
  run: |
    curl -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 \
      -o /usr/local/bin/akmon && chmod +x /usr/local/bin/akmon

- name: Run task
  env:
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
  run: |
    akmon --yes --output json \
      --task "run tests and summarize failures" \
      | jq .result
```

## Uninstalling

```bash
rm /usr/local/bin/akmon
rm -rf ~/.akmon   # removes config, sessions, audit logs
```
