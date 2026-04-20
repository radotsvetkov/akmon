#!/usr/bin/env python3
"""Check local Markdown links deterministically (no network requests)."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Iterable


LINK_RE = re.compile(r"!\[[^\]]*]\(([^)]+)\)|\[[^\]]*]\(([^)]+)\)")
FENCE_RE = re.compile(r"^\s*```")


def iter_markdown_files(paths: list[str], repo_root: Path) -> list[Path]:
    if not paths:
        paths = ["docs/src", "README.md", "CONTRIBUTING.md", "CHANGELOG.md"]
    files: list[Path] = []
    for raw in paths:
        p = Path(raw)
        if not p.is_absolute():
            p = (repo_root / p).resolve()
        if p.is_dir():
            files.extend(sorted(x for x in p.rglob("*.md") if x.is_file()))
        elif p.is_file() and p.suffix.lower() == ".md":
            files.append(p)
    dedup: dict[str, Path] = {}
    for file in files:
        dedup[str(file)] = file
    return sorted(dedup.values())


def normalize_link_target(target: str) -> str:
    t = target.strip()
    if t.startswith("<") and t.endswith(">") and len(t) >= 2:
        t = t[1:-1].strip()
    return t


def should_skip_target(target: str) -> bool:
    lowered = target.lower()
    return (
        not target
        or target.startswith("#")
        or lowered.startswith("http://")
        or lowered.startswith("https://")
        or lowered.startswith("mailto:")
        or lowered.startswith("tel:")
    )


def resolve_target(md_file: Path, target: str, repo_root: Path) -> Path:
    raw = target.split("#", 1)[0].split("?", 1)[0].strip()
    if raw.startswith("/"):
        return (repo_root / raw.lstrip("/")).resolve()
    return (md_file.parent / raw).resolve()


def check_file(md_file: Path, repo_root: Path) -> list[str]:
    errors: list[str] = []
    in_fence = False
    for line_no, line in enumerate(md_file.read_text(encoding="utf-8").splitlines(), start=1):
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for match in LINK_RE.finditer(line):
            target = match.group(1) or match.group(2) or ""
            target = normalize_link_target(target)
            if should_skip_target(target):
                continue
            resolved = resolve_target(md_file, target, repo_root)
            if not resolved.exists():
                errors.append(
                    f"{md_file.relative_to(repo_root)}:{line_no}: broken local link `{target}`"
                )
    return errors


def run(paths: Iterable[str], repo_root: Path) -> int:
    files = iter_markdown_files(list(paths), repo_root)
    if not files:
        print("No markdown files found for link check.", file=sys.stderr)
        return 2
    errors: list[str] = []
    for md_file in files:
        errors.extend(check_file(md_file, repo_root))
    if errors:
        print("Markdown link check failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print(f"Markdown link check passed ({len(files)} files).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Check local markdown links.")
    parser.add_argument("paths", nargs="*", help="Files or directories to scan.")
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    return run(args.paths, repo_root)


if __name__ == "__main__":
    raise SystemExit(main())
