# Installation

Documented for Akmon `2.2.0`.

## Who this is for

Engineers and auditors installing Akmon on macOS or Linux. Two binaries ship from this project:

- `akmon`, the full evidence and verification layer (import, sign, attest, verify, prove, and the bundled reference agent).
- `agef-verify`, a small standalone verifier. An auditor who only needs to check a bundle can install this one binary, without the full Akmon CLI.

If you only receive signed bundles to verify, install `agef-verify` (or use plain `openssl`, see the [quick start](./quickstart.md)). If you produce evidence, install `akmon`.

## What you will have at the end

- An `akmon` binary on `PATH` (and optionally `agef-verify`).
- A verified installation (`akmon --version`).
- A clear fallback from Homebrew to prebuilt binaries to a source build.

## Prerequisites

- Shell access on macOS or Linux.
- `curl` available for the prebuilt-binary path.
- Homebrew for the tap path.
- `Rust 1.88+` only if building from source.
- OpenSSL 3.x only if you intend to verify Ed25519 signatures with plain `openssl`. The macOS system `/usr/bin/openssl` is LibreSSL and cannot verify Ed25519.

## Option 1: Homebrew tap (recommended)

The tap is live. It installs both binaries and keeps them updated through `brew upgrade`.

```bash
brew tap radotsvetkov/akmon
brew install akmon
brew install agef-verify
```

Verify:

```bash
akmon --version
# e.g. akmon 2.2.0
agef-verify --version
```

## Option 2: Prebuilt binaries

Each GitHub release publishes platform binaries for both tools plus a `SHA256SUMS` file. The asset names are `akmon-darwin-arm64`, `akmon-darwin-x86_64`, `akmon-linux-x86_64`, and the matching `agef-verify-*` names. These are slim builds (`--no-default-features`, no bundled semantic index).

### Download and verify the checksum

Always verify the checksum before running a downloaded binary. The release `SHA256SUMS` file is the reference.

**macOS, Apple Silicon**

```bash
mkdir -p ~/bin
base=https://github.com/radotsvetkov/akmon/releases/latest/download
curl -fsSL -L "$base/akmon-darwin-arm64" -o ~/bin/akmon
curl -fsSL -L "$base/SHA256SUMS" -o /tmp/SHA256SUMS
# Confirm the line for akmon-darwin-arm64 matches your file
shasum -a 256 ~/bin/akmon
grep akmon-darwin-arm64 /tmp/SHA256SUMS
chmod +x ~/bin/akmon
```

**macOS, Intel**

```bash
mkdir -p ~/bin
base=https://github.com/radotsvetkov/akmon/releases/latest/download
curl -fsSL -L "$base/akmon-darwin-x86_64" -o ~/bin/akmon
curl -fsSL -L "$base/SHA256SUMS" -o /tmp/SHA256SUMS
shasum -a 256 ~/bin/akmon
grep akmon-darwin-x86_64 /tmp/SHA256SUMS
chmod +x ~/bin/akmon
```

**Linux, x86_64**

```bash
mkdir -p ~/bin
base=https://github.com/radotsvetkov/akmon/releases/latest/download
curl -fsSL -L "$base/akmon-linux-x86_64" -o ~/bin/akmon
curl -fsSL -L "$base/SHA256SUMS" -o /tmp/SHA256SUMS
sha256sum ~/bin/akmon
grep akmon-linux-x86_64 /tmp/SHA256SUMS
chmod +x ~/bin/akmon
```

Install `agef-verify` the same way, substituting the `agef-verify-*` asset name.

**Shell PATH (zsh example)**, if `akmon` is not found:

```bash
echo 'export PATH="$HOME/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### Install to `/usr/local/bin` (needs admin)

```bash
sudo curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/akmon-darwin-arm64 \
  -o /usr/local/bin/akmon && sudo chmod +x /usr/local/bin/akmon
```

Use the correct asset name for your platform, and verify the checksum before first run.

### Troubleshooting downloads

| Symptom | Cause and fix |
|--------|-------------|
| Checksum line does not match | Re-download. A mismatch means a partial download or a tampered file. Do not run it. |
| Permission denied writing to `/usr/local/bin` | Use `~/bin` plus `PATH`, or prefix `sudo` on both `curl` and `chmod`. |
| Small file, or `Not: command not found` when running `akmon` | GitHub returned an HTML error page (often a 404). Confirm a release exists with that asset name. Check with `file ~/bin/akmon`, which should report Mach-O or ELF, not HTML. |
| `curl: (56) Failure writing output` | Destination directory missing or not writable. Run `mkdir -p ~/bin` or fix permissions. |

**Verify**

```bash
akmon --version
# e.g. akmon 2.2.0
```

## Option 3: From source

```bash
git clone https://github.com/radotsvetkov/akmon
cd akmon

# Slim build, no semantic indexing, smaller binary
cargo build --release --no-default-features

# Full build, with semantic indexing
cargo build --release

mkdir -p ~/bin
cp target/release/akmon ~/bin/
cp target/release/agef-verify ~/bin/
```

Or install directly with cargo:

```bash
cargo install --git https://github.com/radotsvetkov/akmon akmon
cargo install --git https://github.com/radotsvetkov/akmon agef-verify
```

## Verification

```bash
command -v akmon
akmon --version
akmon --help
```

Expected result: all commands succeed and print usage or version output.

## Troubleshooting

- If `akmon` is not found, add `~/bin` to `PATH` and restart your shell.
- If a downloaded file is HTML, verify the release asset name and tag availability.
- If `openssl` cannot verify a signature on macOS, you are likely on LibreSSL. Install OpenSSL 3.x (see [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md)).
- For provider failures after install, run `akmon doctor providers`.

## Using over SSH

Akmon is a single static binary. Copy it to any remote machine:

```bash
scp ~/bin/akmon user@remote:~/bin/
ssh user@remote
export PATH="$HOME/bin:$PATH"
akmon --version
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

- name: Verify a bundle
  run: |
    akmon bundle verify build/session.akmon --verify-key signer.pub.hex --require-signature --format json \
      | jq .passed
```

For auditors who only verify, install just the standalone binary:

```yaml
- name: Install agef-verify
  run: |
    sudo curl -fsSL -L https://github.com/radotsvetkov/akmon/releases/latest/download/agef-verify-linux-x86_64 \
      -o /usr/local/bin/agef-verify && sudo chmod +x /usr/local/bin/agef-verify
```

## Uninstalling

```bash
rm -f ~/bin/akmon ~/bin/agef-verify /usr/local/bin/akmon /usr/local/bin/agef-verify
rm -rf ~/.akmon   # removes config, sessions, audit logs
```

If you installed through Homebrew:

```bash
brew uninstall akmon agef-verify
brew untap radotsvetkov/akmon
```
