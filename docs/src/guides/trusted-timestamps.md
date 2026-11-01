# Trusted timestamps

Documented for Akmon `3.0.0`.

## The gap

An Akmon signature proves **which key** signed a session head. It does not prove **when**. The event timestamps inside a bundle come from the producer's clock, and the manifest's `created_at` and `ended_at` are not covered by any signature. A key holder could seal a session today and claim it happened last month.

Where time matters (incident reporting, signed and dated test logs, disputes), add a timestamp from an independent RFC 3161 Time-Stamp Authority (TSA). The TSA signs a hash of your data together with its own clock reading, so anyone can later check that the data existed no later than that time.

Native support, a `manifest.timestamps[]` field planned for AGEF v0.1.4, is **not shipped yet**. Until then, use the recipe below. It needs only OpenSSL 3 and `curl`, and the token is kept next to the bundle.

## Recipe

### 1. Write the signed statement

```bash
akmon bundle prove-openssl audit.akmon --verify-key signer.pub --out-dir proof
```

(or seal with `--proof-dir proof`). `proof/statement.bin` is the exact statement your key signed. It commits to the session id and the session head, and so to every event and object in the bundle.

### 2. Create a timestamp request

```bash
openssl ts -query -data proof/statement.bin -sha256 -cert -out proof/statement.tsq
```

### 3. Send it to a TSA

```bash
curl -sS -H "Content-Type: application/timestamp-query" \
  --data-binary @proof/statement.tsq \
  https://freetsa.org/tsr -o proof/statement.tsr
```

FreeTSA is a free public service, convenient for trying this out. **For legal weight in the EU, use a qualified trust service provider** that issues qualified electronic time stamps under eIDAS (Art. 41, with Implementing Regulation (EU) 2025/1929, which references RFC 3161 and ETSI EN 319 422). A qualified time stamp carries a legal presumption that its date and time are accurate. Find providers on the EU Trusted List.

Do this as soon as you seal. A timestamp proves the head existed no later than the stamped time; it says nothing about earlier.

### 4. Keep the token with the bundle

Store `proof/statement.tsr` next to `audit.akmon` in your archive and send it to verifiers with the bundle. It is not secret.

### 5. Verify (any machine, offline)

Get the TSA's CA certificate (`tsa-ca.pem`) and signing certificate (`tsa.crt`) from the TSA, through a channel you trust. Then:

```bash
openssl ts -verify -data proof/statement.bin -in proof/statement.tsr \
  -CAfile tsa-ca.pem -untrusted tsa.crt
```

`Verification: OK` means the TSA attests that `statement.bin` existed at the time shown by:

```bash
openssl ts -reply -in proof/statement.tsr -text
```

A verifier then also runs `agef-verify audit.akmon --verify-key signer.pub --require-signature`, which ties the statement to the bundle's contents and to your key.

## What this proves, and what it does not

- Proves: the session head (and therefore the bundle contents) existed no later than the TSA's time.
- Does not prove: when the session itself ran, or when the signature was made. Timestamping `proof/signature.bin` the same way adds evidence for the signature time.
- Trust: you now also rely on the TSA's key and clock. Qualified TSAs are supervised under eIDAS; public free services are not.

## See also

- [What verification proves](../concepts/verifying-evidence.md)
- [akmon bundle prove-openssl](../reference/bundle-prove-openssl.md)
- [Compliance crosswalk](../concepts/compliance.md)
