# Hand evidence to an auditor

Documented for Akmon `3.0.0`.

You have a sealed bundle. This page covers what to send, how to send it, and what to tell the person who checks it.

## What to send

| Item | How to send it | Why |
| --- | --- | --- |
| `audit.akmon` | Any channel: email, file share, ticket attachment. | The evidence. It is tamper-evident, so the channel does not need to be trusted. |
| The signer public key (`audit.akmon.pub` or `signer.pub`) | A **separate, trusted** channel: your website, a signed email, a key registered in their system, or read out on a call. | If the bundle and key travel together, whoever can replace one can replace both. |
| The operator public key, if you attested | Same as the signer key. | Lets them check who approved. |
| `proof/` directory (optional) | With the bundle. | Lets them check the signature with OpenSSL alone. |
| Timestamp token `.tsr` (optional) | With the bundle. | Proves when the session head existed. See [Trusted timestamps](./trusted-timestamps.md). |

For a long-running relationship, register your signer public key with the auditor once, and send only bundles afterwards.

## What to tell them

A short cover note is enough:

> Attached is `audit.akmon`, a signed record of the agent session for change 1234. Our signing key's fingerprint (key_id) is `327f8f3b…771f`; the public key is published at our usual location. To verify, install `agef-verify` (Homebrew `radotsvetkov/akmon` tap, or the release binary with `SHA256SUMS`) and run:
>
> `agef-verify audit.akmon --verify-key signer.pub --require-signature --format markdown > report.md`
>
> The session's capture level is `full`. No network access is needed.

The key_id is printed by `akmon bundle keygen` and `akmon seal`. Giving it in the note lets the auditor confirm they hold the right key.

## What the auditor does

```bash
agef-verify audit.akmon --verify-key signer.pub --require-signature
```

- Exit `0` with a headline starting `verified bundle: integrity OK; signature verified by key_id=…` means the bundle is intact and your key signed it.
- `--format markdown` produces a report with a verdict, a checks table, and plain-language "What this proves" and "What this does not prove" sections they can attach to their file.
- If they do not want to run any Akmon software, they run the OpenSSL command from the `proof/` directory. That proves the key signed head H; `agef-verify` is what ties H to the contents.

Point them to [What verification proves](../concepts/verifying-evidence.md) so the result is not over-read: it does not prove when the session happened (unless you sent a timestamp), who holds the key, or that the agent was right.

## If the bundle contains sensitive content

A `full` capture contains prompts and responses. Before sending outside your organization:

1. Create a redacted derivative with [`akmon redact`](../reference/redact.md). Redacted objects are replaced by sentinels that record the reason, and the structure stays verifiable.
2. Sign the derivative (`akmon bundle sign`), since it has a new head.
3. Keep the original sealed bundle under retention, so you can produce it to an authority if required.

## See also

- [Verify on an air-gapped machine](../use-cases/air-gapped-audit.md)
- [agef-verify](../reference/agef-verify.md)
- [Retention and archiving](./retention.md)
