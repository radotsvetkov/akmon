# akmon bundle keygen

Documented for Akmon `2.2.0`.

## Who this is for

Anyone who needs to sign an Akmon bundle. `akmon bundle sign` requires an Ed25519 private key in
raw **PKCS#8 v2 DER** form, and this command is the supported way to create one. Without it there is
no first-class way to make a usable signing key, and `openssl genpkey` does **not** fill the gap
(see the honesty note below).

## What you will have at the end

- A PKCS#8 v2 DER private key at `--out` (raw bytes, no PEM armor), created with `0600` permissions
  on unix. This is the exact byte form `akmon bundle sign --key` consumes.
- The signer's **public key** as 64 hex characters, surfaced on stderr (human mode) or in the JSON
  report, and optionally written to `--public-out`.
- The signer's **key_id** (lowercase hex SHA-256 of the public key), the same value recorded in
  `manifest.signatures[].key_id`.

Distribute the public key (hex) to verifiers; they use it with `akmon bundle verify --verify-key`
or `akmon bundle prove-openssl --verify-key`. Keep the private key secret.

## How it works

`keygen` generates a fresh Ed25519 keypair, writes the private key (PKCS#8 v2 DER) to `--out`, and
derives the raw 32-byte public key from it. It never writes a bundle, manifest, or any signature.
it only produces the key material. The private key is written via a file opened with mode `0600`
(owner read/write only) at create time on unix, so there is no window where the key exists with
broader permissions.

## Steps

Generate a key:

```bash
akmon bundle keygen --out signer.pk8
```

Generate a key and also write the public key for verifiers:

```bash
akmon bundle keygen --out signer.pk8 --public-out signer.pub.hex
```

Then sign and verify a bundle:

```bash
akmon bundle sign /path/to/audit.akmon --key signer.pk8
akmon bundle verify /path/to/audit.akmon --verify-key signer.pub.hex --require-signature
```

## Optional flags

- `--public-out <FILE>`: also write the public key as exactly 64 hex characters (no trailing
  newline) to this file, ready for `--verify-key`.
- `--force`: allow overwriting an existing `--out` (and `--public-out`). Off by default: keygen
  refuses to clobber an existing private key.
- `--format human|json`: default `human`. JSON emits **KeygenReportV1** with `tool`,
  `akmon_version`, `key_path`, `public_out` (or `null`), `public_key_hex`, and `key_id`.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Key written; public key hex and key_id surfaced |
| `3` | I/O error, refuse-to-clobber (pass `--force` to replace), or key generation failure |

## Security notes

- **Keep the private key secret.** Anyone holding it can forge signatures attributed to you. Only
  ever distribute the public key (hex).
- On unix the private key is created with `0600` permissions at create time (never a broader-then-
  narrowed window). A `--force` overwrite re-asserts `0600` on the file before any bytes are
  written.
- On Windows there is no `0600` enforcement; the file inherits the parent directory's NTFS ACLs.
  Store the key in a directory that only you can read.

## Honesty note: openssl is not a substitute

`openssl genpkey -algorithm ed25519` (even with `-outform DER`) emits a PKCS#8 **v1** key, which the
`ring` library Akmon uses **rejects**, so such a key cannot sign an Akmon bundle. Use
`akmon bundle keygen` to produce a usable PKCS#8 v2 key.

## See also

- [akmon sign](./sign.md)
- [akmon bundle verify](./bundle-verify.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
- [agef-verify](./agef-verify.md)
