#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

echo "==> Fixture test: link checker positive"
python3 scripts/docs/check_markdown_links.py docs/tests/fixtures/links/pass.md

echo "==> Fixture test: link checker negative"
if python3 scripts/docs/check_markdown_links.py docs/tests/fixtures/links/fail_broken.md; then
  echo "expected broken link fixture to fail" >&2
  exit 1
fi

echo "==> Fixture test: json checker positive"
python3 scripts/docs/check_json_snippets.py docs/tests/fixtures/json/pass.md

echo "==> Fixture test: json checker negative"
if python3 scripts/docs/check_json_snippets.py docs/tests/fixtures/json/fail_malformed.md; then
  echo "expected malformed json fixture to fail" >&2
  exit 1
fi

echo "Fixture tests passed."
