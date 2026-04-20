#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

echo "Running CLI docs smoke checks..."

cargo run -p akmon-cli --no-default-features -- --help >/dev/null
cargo run -p akmon-cli --no-default-features -- audit verify --help >/dev/null
cargo run -p akmon-cli --no-default-features -- evidence verify --help >/dev/null
cargo run -p akmon-cli --no-default-features -- slo verify --help >/dev/null
cargo run -p akmon-cli --no-default-features -- slo trend --help >/dev/null
cargo run -p akmon-cli --no-default-features -- doctor providers --help >/dev/null

echo "CLI docs smoke checks passed."
