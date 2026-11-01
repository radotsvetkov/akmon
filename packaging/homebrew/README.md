# Homebrew formulas for Akmon

These are the formulas for the [`radotsvetkov/homebrew-akmon`](https://github.com/radotsvetkov/homebrew-akmon) tap. Each one installs the prebuilt binary from the matching Akmon GitHub release and checks its SHA-256. They live in this repository so that the source of truth is reviewed and versioned alongside the code.

- `Formula/akmon.rb`: the `akmon` CLI (seal, bundle, otel import, verify, …).
- `Formula/agef-verify.rb`: the standalone verifier. This is all an auditor needs.

```bash
brew tap radotsvetkov/akmon
brew install akmon agef-verify
```

## Updating for a new release

The formulas are bumped by hand after each release. The release workflow's run summary ("Homebrew tap update needed") lists the exact `sha256` values to use.

1. Wait for the `Release` workflow for tag `vX.Y.Z` to finish, and download that release's `SHA256SUMS`.
2. In both formulas, set `version "X.Y.Z"`, update every release URL to `vX.Y.Z`, set each `sha256` from the matching `SHA256SUMS` line, and update the version asserted in the `test do` block.
3. Validate locally:

   ```bash
   brew style packaging/homebrew/Formula/*.rb
   brew install --formula ./packaging/homebrew/Formula/agef-verify.rb && agef-verify --version
   brew install --formula ./packaging/homebrew/Formula/akmon.rb && akmon --version
   ```

4. Commit the change here, then copy `Formula/*.rb` into the tap repository and push it.

Until step 4 is done, `brew upgrade` keeps users on the previous version. The prebuilt binaries and `cargo install` paths in the main README always have the latest release.

## Bootstrapping the tap (one-time)

A Homebrew tap lives in its own repository named `homebrew-<name>`:

```bash
tmp=$(mktemp -d)
mkdir -p "$tmp/Formula"
cp packaging/homebrew/Formula/*.rb "$tmp/Formula/"
cd "$tmp"
git init -b main
git add -A
git commit -s -m "Add Akmon tap: akmon and agef-verify formulas"
gh repo create radotsvetkov/homebrew-akmon --public --source=. --push \
  -d "Homebrew tap for Akmon (akmon CLI + agef-verify)"
```
