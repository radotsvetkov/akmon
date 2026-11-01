#!/usr/bin/env python3
"""Check that user-facing version markers match the workspace version.

The single source of truth is `version` under `[workspace.package]` in the root
`Cargo.toml`. This script fails when:

- a required handbook page has no "Documented for Akmon `X.Y.Z`" marker;
- any "Documented for Akmon `X.Y.Z`" marker under docs/src names another version;
- README.md or landing/index.html does not name the current version, or the
  landing page advertises a different version in its nav badge or JSON-LD;
- the release notes page docs/src/releases/v<version>.md is missing.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

REQUIRED_FILES = [
    "docs/src/getting-started/installation.md",
    "docs/src/getting-started/quickstart.md",
    "docs/src/getting-started/configuration.md",
    "docs/src/tutorials/local-first-ollama.md",
    "docs/src/tutorials/ci-headless-governance.md",
    "docs/src/tutorials/enterprise-policy-rollout.md",
    "docs/src/features/evidence.md",
    "docs/src/reference/cli.md",
    "docs/src/reference/config.md",
    "docs/src/reference/env-vars.md",
    "docs/src/reference/verify.md",
    "docs/src/reference/replay.md",
    "docs/src/reference/bundle-export.md",
    "docs/src/reference/bundle-import.md",
    "docs/src/reference/redact.md",
    "docs/src/reference/slash-commands.md",
    "docs/src/concepts/glossary.md",
    "docs/src/concepts/reviewer-flow.md",
    "docs/src/concepts/trust-model.md",
    "docs/src/concepts/architecture.md",
    "docs/src/concepts/verifying-evidence.md",
    "docs/src/concepts/compliance.md",
    "docs/src/use-cases/operator-sign-off.md",
    "docs/src/use-cases/air-gapped-audit.md",
    "docs/src/use-cases/release-evidence-pack.md",
]

MARKER_RE = re.compile(r"Documented for Akmon `?v?(\d+\.\d+\.\d+)`?")
SEMVER = r"\d+\.\d+\.\d+"


def workspace_version() -> str:
    text = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    in_section = False
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("["):
            in_section = line == "[workspace.package]"
            continue
        if in_section:
            m = re.match(r'version\s*=\s*"([^"]+)"', line)
            if m:
                return m.group(1)
    raise SystemExit("check_version_markers: no version under [workspace.package] in Cargo.toml")


def main() -> int:
    version = workspace_version()
    problems: list[str] = []

    for rel in REQUIRED_FILES:
        path = REPO_ROOT / rel
        if not path.is_file():
            problems.append(f"{rel}: file missing")
        elif not MARKER_RE.search(path.read_text(encoding="utf-8")):
            problems.append(f"{rel}: no 'Documented for Akmon `{version}`' marker")

    checked = 0
    for path in sorted((REPO_ROOT / "docs" / "src").rglob("*.md")):
        rel = path.relative_to(REPO_ROOT).as_posix()
        if rel.startswith("docs/src/releases/"):
            continue  # historical release notes keep their own versions
        for found in MARKER_RE.findall(path.read_text(encoding="utf-8")):
            checked += 1
            if found != version:
                problems.append(f"{rel}: marker says {found}, workspace is {version}")

    readme = (REPO_ROOT / "README.md").read_text(encoding="utf-8")
    if f"v{version}" not in readme:
        problems.append(f"README.md: does not mention v{version}")

    landing_path = REPO_ROOT / "landing" / "index.html"
    landing = landing_path.read_text(encoding="utf-8")
    nav = re.search(rf'class="nav-version">v({SEMVER})<', landing)
    if not nav:
        problems.append("landing/index.html: nav version badge not found")
    elif nav.group(1) != version:
        problems.append(f"landing/index.html: nav badge says v{nav.group(1)}, workspace is {version}")
    ld = re.search(rf'"softwareVersion":\s*"({SEMVER})"', landing)
    if ld and ld.group(1) != version:
        problems.append(f"landing/index.html: JSON-LD softwareVersion {ld.group(1)}, workspace is {version}")
    for m in re.finditer(rf"#\s*akmon ({SEMVER})", landing):
        if m.group(1) != version:
            problems.append(f"landing/index.html: example output says akmon {m.group(1)}")
    for m in re.finditer(rf"releases/v({SEMVER})\.html", landing):
        if m.group(1) != version:
            problems.append(f"landing/index.html: links release notes v{m.group(1)}, not the current v{version}")

    notes = REPO_ROOT / "docs" / "src" / "releases" / f"v{version}.md"
    if not notes.is_file():
        problems.append(f"{notes.relative_to(REPO_ROOT).as_posix()}: release notes missing")

    if problems:
        print(f"Version marker check failed (workspace version {version}):")
        for item in problems:
            print(f"- {item}")
        return 1

    print(
        f"Version marker check passed: workspace {version}, "
        f"{len(REQUIRED_FILES)} required pages, {checked} markers, README, landing, release notes."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
