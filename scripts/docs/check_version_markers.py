#!/usr/bin/env python3
from pathlib import Path
import sys


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

MARKER_PREFIX = "Documented for Akmon "


def main() -> int:
    missing = []
    for rel in REQUIRED_FILES:
        path = REPO_ROOT / rel
        if not path.is_file():
            missing.append(f"{rel} (file missing)")
            continue
        text = path.read_text(encoding="utf-8")
        if MARKER_PREFIX not in text:
            missing.append(rel)

    if missing:
        print("Version marker check failed. Missing marker in:")
        for item in missing:
            print(f"- {item}")
        return 1

    print(f"Version marker check passed ({len(REQUIRED_FILES)} files).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
