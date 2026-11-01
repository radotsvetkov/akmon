# akmon bundle sign

Documented for Akmon `3.0.0`.

Add an Ed25519 signature over a bundle's session head. [`akmon seal`](./seal.md) does this for you; use `bundle sign` when you export and sign in separate steps, or to co-sign a bundle someone else sealed.

## Usage

```bash
akmon bundle sign audit.akmon --key signer.pk8
akmon bundle sign audit.akmon --key signer.pk8 --output signed.akmon
akmon bundle sign audit.akmon --key signer.pk8 --format json
```

| Option | Meaning |
| --- | --- |
| `--key <FILE>` | PKCS#8 v2 Ed25519 private key (raw DER) from [`akmon bundle keygen`](./bundle-keygen.md). Required. |
| `--output <PATH>` | Write the signed bundle here. Default: sign in place. |
| `--format human\|json` | Output format. Default `human`. |

## What it does

1. Reads the bundle and verifies its integrity. A bundle whose contents do not match its head is refused (category `integrity_failed`, exit `1`) and left untouched, so you never sign a head that commits to nothing.
2. Signs the canonical `AGEF-SIG-v1` statement over the session head.
3. Appends the detached signature to `manifest.signatures[]`.
4. Prints the signer's public key in hex and the `agef-verify` command to give to verifiers.

Signing is additive. Existing signatures and operator attestations stay valid; `agef_version` is only stamped on a bundle that has no signatures and no attestations yet. Several parties can co-sign the same bundle.

The signature covers the session head. It does not cover the manifest's `created_at`, `ended_at`, or producer fields, and it does not cover a time. See [Trusted timestamps](../guides/trusted-timestamps.md).

## Keys

Generate keys with `akmon bundle keygen`. `openssl genpkey -algorithm ed25519` writes PKCS#8 v1, which Akmon rejects (exit `2`).

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Signed. |
| `1` | Refused: integrity failed, or the archive is malformed or corrupt. |
| `2` | Usage error, for example an invalid key. |
| `3` | I/O or environment error. |

Errors in `--format json` use the shared error object `{tool, akmon_version, passed: false, error, category}`.

## See also

- [akmon bundle keygen](./bundle-keygen.md)
- [akmon bundle verify](./bundle-verify.md)
- [akmon bundle attest](./bundle-attest.md)
- [akmon seal](./seal.md)
