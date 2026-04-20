#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

echo "==> mdBook build"
mdbook build docs

echo "==> Markdown link checks"
python3 scripts/docs/check_markdown_links.py

echo "==> CLI smoke checks"
bash scripts/docs/check_cli_smoke.sh

echo "==> JSON snippet sanity checks"
python3 scripts/docs/check_json_snippets.py

echo "All docs checks passed."
