#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

echo "Running CLI docs smoke checks..."

cargo run -p akmon-cli --no-default-features -- --help >/dev/null
cargo run -p akmon-cli --no-default-features -- audit verify --help >/dev/null
cargo run -p akmon-cli --no-default-features -- evidence verify --help >/dev/null
cargo run -p akmon-cli --no-default-features -- doctor providers --help >/dev/null

# Trust path: every command the README, landing page and handbook lead with.
cargo run -p akmon-cli --no-default-features -- seal --help >/dev/null
cargo run -p akmon-cli --no-default-features -- otel import --help >/dev/null
cargo run -p akmon-cli --no-default-features -- bundle keygen --help >/dev/null
cargo run -p akmon-cli --no-default-features -- bundle export --help >/dev/null
cargo run -p akmon-cli --no-default-features -- bundle sign --help >/dev/null
cargo run -p akmon-cli --no-default-features -- bundle attest --help >/dev/null
cargo run -p akmon-cli --no-default-features -- bundle verify --help >/dev/null
cargo run -p akmon-cli --no-default-features -- bundle prove-openssl --help >/dev/null
cargo run -p agef-verify -- --help >/dev/null

echo "CLI docs smoke checks passed."
