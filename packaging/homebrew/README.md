# Homebrew formulas for Akmon

These are the validated formulas for the `radotsvetkov/homebrew-akmon` tap. Each installs the
prebuilt binary from the matching Akmon GitHub release and verifies its SHA-256. They are kept here
in-tree so the source of truth is reviewed and versioned alongside the code.

Validated against the v2.2.0 release with `brew style` (clean), `brew fetch` (checksums match), and a
`brew install` smoke test.

## Publishing the tap

A Homebrew tap lives in its own repository named `homebrew-<name>`. To stand it up from these files:

```bash
tmp=$(mktemp -d)
mkdir -p "$tmp/Formula"
cp packaging/homebrew/Formula/*.rb "$tmp/Formula/"
cd "$tmp"
git init -b main
git add -A
git commit -m "Add Akmon tap: akmon and agef-verify formulas (v2.2.0)"
gh repo create radotsvetkov/homebrew-akmon --public --source=. --push \
  -d "Homebrew tap for Akmon (akmon CLI + agef-verify)"
```

Then anyone can install:

```bash
brew tap radotsvetkov/akmon
brew install akmon
brew install agef-verify
```

## Updating for a new release

Bump `version` and the `sha256` values in both formulas to match the new release's `SHA256SUMS`, then
push to the tap repo. This is a good candidate to automate from the release workflow later.
