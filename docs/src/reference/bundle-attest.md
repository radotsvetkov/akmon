# akmon bundle attest

Documented for Akmon `2.1.0`.

## Who this is for

Operators who want to bind a **named human (or service account) and a role** to an AGEF bundle's
session — answering "who claims to have operated this session" — on top of the bundle's integrity
hash chain and any head signature. `akmon bundle attest` records a signed `AGEF-OPERATOR-v1`
operator attestation in `manifest.operator_attestations[]`. Verifiers later check it with
[`akmon bundle verify --operator-key`](./bundle-verify.md) or
[`agef-verify --operator-key`](./agef-verify.md).

## The honesty point: trust attaches to the KEY, not the name

An attestation is a signature over the four self-asserted identity fields (`operator_id`,
`display_name`, `role`, `org`) plus the session head. Verification proves only that **the holder of
a particular private key signed those fields** — it does **not** prove the person is who the
`operator_id` string claims. Trust in the identity is **out-of-band**: a verifier decides which
`key_id` (public key) they trust, by some external process (an HR directory, a key-distribution
ceremony, a signed roster), and only then does the self-asserted name carry weight. Verification
surfaces the name verbatim but the trust signal is `operator_key_verified` against a key the
verifier supplied. Akmon never claims the name is true on its own.

## What you will have at the end

- The same bundle with one more entry appended to `manifest.operator_attestations[]`. The write is
  atomic (temp file + rename) and **purely additive**: the event hash chain, the `AGEF-SIG-v1` head
  statement, any existing head signatures, and the `prove-openssl` head bytes are byte-untouched.
- The attester's **key_id** (lowercase hex SHA-256 of the operator public key) and the operator
  **public key** as 64 hex characters, surfaced on stderr (human mode) or in the JSON report.

## Prerequisites

- A `.akmon` bundle on disk (signed or unsigned).
- An Ed25519 private key in raw **PKCS#8 v2 DER** form, as produced by
  [`akmon bundle keygen --out`](./bundle-keygen.md). (`openssl genpkey` emits PKCS#8 v1, which the
  signing path rejects — see the keygen honesty note.)
- A stable `--operator-id` (an email, employee id, or service account). It is required and must not
  contain a newline or carriage return.

## Steps

Generate an operator key and attest a bundle in place:

```bash
akmon bundle keygen --out operator.pk8 --public-out operator.pub.hex
akmon bundle attest /path/to/audit.akmon --key operator.pk8 --operator-id ops@example.com --role approver
```

Then distribute `operator.pub.hex` to verifiers (out-of-band) and have them verify:

```bash
akmon bundle verify /path/to/audit.akmon --operator-key operator.pub.hex --require-operator
```

## Optional flags

- `--display-name <NAME>` — human-readable display name (signed). Defaults to empty.
- `--role <ROLE>` — role the operator acted in, for example `approver` (signed). Defaults to empty.
- `--org <ORG>` — organization the operator belongs to (signed). Defaults to empty.
- `--output <FILE>` — write the attested bundle here instead of attesting in place.
- `--format human|json` — default `human`. JSON emits **BundleAttestReportV1** with `tool`,
  `akmon_version`, `bundle_path`, `session_id`, `operator_id`, `role`, `key_id`, `public_key_hex`,
  and `output_path`. The private key is never printed.

## Note on head signatures (O9)

If the bundle already carries a head signature, `attest` leaves `agef_version` untouched so the
existing `AGEF-SIG-v1` signature stays valid; on an unsigned bundle it stamps the current spec
version. Either way the attestation is built from the manifest's current `agef_version`, so it is
self-consistent. Attesting never invalidates a previously verifiable head signature.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Attestation appended and bundle written |
| `1` | Bundle read or integrity error |
| `2` | Invalid private key, or an operator identity field is empty (`operator-id`) or contains a newline/carriage return |
| `3` | I/O error (bundle unreadable, key unreadable, or bundle write/rename failed) |

## See also

- [akmon bundle keygen](./bundle-keygen.md)
- [akmon bundle verify](./bundle-verify.md)
- [akmon bundle prove-openssl](./bundle-prove-openssl.md)
- [agef-verify](./agef-verify.md)
- [akmon sign](./sign.md)
