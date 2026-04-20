#!/usr/bin/env python3
"""Validate fenced JSON snippets in selected docs files."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


FENCE_JSON_RE = re.compile(r"^\s*```json\s*$", re.IGNORECASE)
FENCE_END_RE = re.compile(r"^\s*```\s*$")


def iter_files(paths: list[str], repo_root: Path) -> list[Path]:
    if not paths:
        paths = [
            "docs/src/reference/cli.md",
            "docs/src/features/evidence.md",
            "docs/src/features/audit-log.md",
        ]
    out: list[Path] = []
    for raw in paths:
        p = Path(raw)
        if not p.is_absolute():
            p = (repo_root / p).resolve()
        if p.is_file():
            out.append(p)
    return sorted(out)


def validate_file(path: Path, repo_root: Path) -> list[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    i = 0
    errors: list[str] = []
    while i < len(lines):
        if not FENCE_JSON_RE.match(lines[i]):
            i += 1
            continue
        start_line = i + 1
        i += 1
        block: list[str] = []
        while i < len(lines) and not FENCE_END_RE.match(lines[i]):
            block.append(lines[i])
            i += 1
        payload = "\n".join(block).strip()
        if not payload:
            errors.append(f"{path.relative_to(repo_root)}:{start_line}: empty json code block")
        else:
            if not is_valid_json_payload(payload):
                errors.append(
                    f"{path.relative_to(repo_root)}:{start_line}: invalid json snippet"
                )
        if i < len(lines) and FENCE_END_RE.match(lines[i]):
            i += 1
    return errors


def is_valid_json_payload(payload: str) -> bool:
    try:
        json.loads(payload)
        return True
    except json.JSONDecodeError:
        pass
    non_empty_lines = [line.strip() for line in payload.splitlines() if line.strip()]
    if not non_empty_lines:
        return False
    for line in non_empty_lines:
        try:
            json.loads(line)
        except json.JSONDecodeError:
            return False
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description="Check fenced JSON snippets.")
    parser.add_argument("paths", nargs="*", help="Specific markdown files to validate.")
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    files = iter_files(args.paths, repo_root)
    if not files:
        print("No markdown files found for JSON snippet check.", file=sys.stderr)
        return 2
    errors: list[str] = []
    for f in files:
        errors.extend(validate_file(f, repo_root))
    if errors:
        print("JSON snippet check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print(f"JSON snippet check passed ({len(files)} files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
