# Installation

## Requirements

- **Git** — for project context and git operations
- **Rust 1.88+** — only if building from source
- **Ollama** — optional, for local offline models

## Option 1 — Pre-built binary (recommended)

Releases on GitHub include **`akmon-darwin-arm64`**, **`akmon-darwin-x86_64`**, and **`akmon-linux-x86_64`**. They are slim builds (`--no-default-features`, no bundled semantic index).

### Install without `sudo` (recommended)

Put the binary in **`~/bin`** and ensure it is on your `PATH`:

**macOS — Apple Silicon**
```bash
mkdir -p ~/bin
curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-darwin-arm64 \
  -o ~/bin/akmon && chmod +x ~/bin/akmon
```

**macOS — Intel**
```bash
mkdir -p ~/bin
curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-darwin-x86_64 \
  -o ~/bin/akmon && chmod +x ~/bin/akmon
```

**Linux — x86_64**
```bash
mkdir -p ~/bin
curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 \
  -o ~/bin/akmon && chmod +x ~/bin/akmon
```

**Shell PATH (zsh example)** — if `akmon` is not found:
```bash
echo 'export PATH="$HOME/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### Install to `/usr/local/bin` (needs admin)

```bash
sudo curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-darwin-arm64 \
  -o /usr/local/bin/akmon && sudo chmod +x /usr/local/bin/akmon
```

(Use the correct asset name for your platform.)

### Troubleshooting downloads

| Symptom | Cause / fix |
|--------|-------------|
| **Permission denied** writing to `/usr/local/bin` | Use `~/bin` + `PATH`, or prefix `sudo` on both `curl` and `chmod`. |
| **Small file / `Not: command not found`** when running `akmon` | GitHub returned an HTML error page (often **404**). Ensure a release exists with that asset name (tag the repo so the [release workflow](https://github.com/radotsvetkov/akmon/blob/main/.github/workflows/release.yml) uploads binaries). Check with `file ~/bin/akmon` — it should say “Mach-O” or “ELF”, not “HTML”. |
| **`curl: (56) Failure writing output`** | Destination directory missing or not writable; use `mkdir -p ~/bin` or fix permissions. |

**Verify**
```bash
akmon --version
# e.g. akmon 1.8.0
```

## Option 2 — From source

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon

# Slim build — no semantic indexing, smaller binary
cargo build --release --no-default-features

# Full build — with semantic indexing
cargo build --release

mkdir -p ~/bin
cp target/release/akmon ~/bin/
```

## Option 3 — cargo install

```bash
cargo install akmon
```

## Using over SSH

Akmon is a single static binary. Copy it to any remote machine:

```bash
scp ~/bin/akmon user@remote:~/bin/
ssh user@remote
export PATH="$HOME/bin:$PATH"
akmon chat
```

## Using in Docker

```dockerfile
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/* \
  && curl -fsSL -L \
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
    sudo curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-linux-x86_64 \
      -o /usr/local/bin/akmon && sudo chmod +x /usr/local/bin/akmon

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
rm -f ~/bin/akmon /usr/local/bin/akmon
rm -rf ~/.akmon   # removes config, sessions, audit logs
```
