# Retention and archiving

Documented for Akmon `3.0.0`.

A sealed bundle stays verifiable for as long as you keep it. Akmon does not store, retain, or delete anything for you; this page is about keeping the evidence safely.

## How long to keep it

Akmon cannot tell you your retention period. Some reference points:

- **EU AI Act, high-risk systems.** Providers (Art. 19) and deployers (Art. 26(6)) must keep automatically generated logs for **at least six months**, unless other Union or national law requires otherwise. These obligations apply from 2 December 2027 for Annex III systems and 2 August 2028 for Annex I systems.
- **DORA** (financial entities). The ICT logging rules in RTS (EU) 2024/1774 require you to define and apply a retention period for logs. Sector rules and supervisors often expect several years.
- **Incidents.** Keep everything related to a serious incident until the investigation and any reporting are closed. Under the AI Act (Art. 73(6)), a provider must not alter the system in a way that could affect later evaluation of the incident's causes before informing the authorities; keeping the records sealed and untouched is part of that.
- **Contracts and audits** may require longer.

Do not delete agent logs on a short default schedule (for example 30 days) just because a tool does so by default. Decide the period with your legal and compliance teams, write it down, and apply it.

## What to archive

For each sealed session:

- the bundle (`audit.akmon`),
- the signer public key (`audit.akmon.pub`) and any operator public key,
- the `proof/` directory and any timestamp token (`.tsr`), if you made them,
- a copy of the exact `agef-verify` binary version and its `SHA256SUMS`, so the evidence can be checked years later without depending on a download.

Keep the private signing keys **out** of the archive.

## Write-once storage

Storing bundles where they cannot be changed or deleted before the retention period ends strengthens the evidence: it shows nobody could have quietly replaced or removed one. The bundle already detects changes; immutable storage prevents deletion.

### Amazon S3 Object Lock

```bash
aws s3api create-bucket --bucket acme-akmon-evidence --object-lock-enabled-for-bucket

aws s3api put-object-lock-configuration --bucket acme-akmon-evidence \
  --object-lock-configuration \
  '{"ObjectLockEnabled":"Enabled","Rule":{"DefaultRetention":{"Mode":"COMPLIANCE","Days":365}}}'

aws s3 cp audit.akmon     s3://acme-akmon-evidence/2026/11/audit.akmon
aws s3 cp audit.akmon.pub s3://acme-akmon-evidence/2026/11/audit.akmon.pub
```

`COMPLIANCE` mode means nobody, including the account root user, can delete an object or shorten its retention until it expires. Use `GOVERNANCE` while you are testing. Outside `us-east-1`, add `--create-bucket-configuration LocationConstraint=<region>` to `create-bucket`.

### Azure Blob Storage immutability policy

```bash
az storage container create --account-name acmeevidence --name akmon-evidence

az storage container immutability-policy create \
  --account-name acmeevidence --container-name akmon-evidence --period 365

# After testing, lock the policy (irreversible); use the etag returned by the create call:
az storage container immutability-policy lock \
  --account-name acmeevidence --container-name akmon-evidence --if-match "<etag>"

az storage blob upload --account-name acmeevidence --container-name akmon-evidence \
  --name 2026/11/audit.akmon --file audit.akmon
```

A locked time-based retention policy prevents blobs from being modified or deleted until the period ends.

### On-premises

Use WORM-capable storage (an appliance, object store with retention lock, or WORM tape). Setting files read-only or immutable at the file-system level (for example `chattr +i` on Linux) helps against accidents, but an administrator can undo it, so it is weaker evidence.

## Check the archive regularly

Verification is cheap and offline. Re-verify archived bundles on a schedule, and before you hand any of them over:

```bash
for b in archive/*.akmon; do
  agef-verify "$b" --verify-key "$b.pub" --require-signature --format json > "$b.verify.json" \
    || echo "FAILED: $b"
done
```

## Keys over time

- Keep a record of which key_id belonged to which team and when it was in use. Verifiers need the right public key years later.
- If a signing key is compromised, stop using it and tell your verifiers. Akmon has no revocation list; bundles timestamped (see [Trusted timestamps](./trusted-timestamps.md)) before the compromise remain more credible than untimestamped ones.

## See also

- [Compliance crosswalk](../concepts/compliance.md)
- [Hand evidence to an auditor](./hand-to-auditor.md)
