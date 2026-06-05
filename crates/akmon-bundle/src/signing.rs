//! AGEF v0.1.2 detached session signatures (Ed25519 via `ring`).
//!
//! A signature authenticates the *session head* — the merkle root that commits to every event and
//! object — without ever entering the hash chain (decision D-18; AGEF v0.1.2 §A.14). Signers sign a
//! canonical, domain-separated [`signing_statement`] rather than the bare head hash, so a signature
//! cannot be replayed against a different session, hash algorithm, or protocol.
//!
//! The scheme is Ed25519 (RFC 8032) via `ring`, which is already in Akmon's dependency tree (via
//! rustls) — no new supply-chain surface. Keys are PKCS#8 v2 (private) / raw 32-byte (public);
//! signatures are 64 bytes, stored as lowercase hex in `manifest.signatures[]`.

use ring::rand::SystemRandom;
use ring::signature::{self, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

/// Version tag of the canonical signing statement (AGEF v0.1.2 §A.14).
pub const SIG_STATEMENT_VERSION: &str = "AGEF-SIG-v1";

/// Scheme identifier recorded in `manifest.signatures[].scheme` for Ed25519 signatures.
pub const SCHEME_ED25519: &str = "ed25519";

/// Length in bytes of a raw Ed25519 public key.
const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Errors from signing, verification, or key handling.
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    /// The PKCS#8 bytes could not be parsed as an Ed25519 private key.
    #[error("invalid PKCS#8 Ed25519 private key")]
    InvalidPrivateKey,
    /// The public key was not a 32-byte Ed25519 public key.
    #[error("invalid Ed25519 public key: expected 32 bytes, got {0}")]
    InvalidPublicKey(usize),
    /// A public key supplied as a hex string could not be decoded.
    #[error("public key is not valid hex")]
    MalformedPublicKeyHex,
    /// The system random number generator could not produce a key.
    #[error("Ed25519 key generation failed")]
    KeyGeneration,
    /// Verification failed: wrong key, malformed signature, or a tampered statement.
    #[error("Ed25519 signature verification failed")]
    VerificationFailed,
    /// An operator-identity field was empty or contained a `\n`/`\r`, which would break the
    /// line-oriented `AGEF-OPERATOR-v1` statement framing (decision D-20; an injection guard).
    #[error("operator identity field contains an illegal newline or is empty")]
    IllegalOperatorField,
}

/// Builds the canonical `AGEF-SIG-v1` statement that a signature covers.
///
/// Fixed field order, LF line endings, single trailing newline, no other whitespace
/// (AGEF v0.1.2 §A.14). Callers pass the manifest's own `agef_version`, `hash_algorithm`,
/// hyphenated `session_id`, and lowercase-hex `head` verbatim.
#[must_use]
pub fn signing_statement(
    agef_version: &str,
    hash_algorithm: &str,
    session_id: &str,
    head_hex: &str,
) -> String {
    format!(
        "{SIG_STATEMENT_VERSION}\n\
         agef_version:{agef_version}\n\
         hash_algorithm:{hash_algorithm}\n\
         session_id:{session_id}\n\
         head:{head_hex}\n"
    )
}

/// Generates a fresh Ed25519 keypair, returning PKCS#8 v2 private-key bytes.
///
/// The bytes interoperate with `openssl pkey -inform DER` and round-trip through
/// [`public_key_from_pkcs8`] and [`sign_statement`].
pub fn generate_pkcs8() -> Result<Vec<u8>, SigningError> {
    let rng = SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| SigningError::KeyGeneration)?;
    Ok(doc.as_ref().to_vec())
}

/// Returns the raw 32-byte Ed25519 public key for the given PKCS#8 private key.
pub fn public_key_from_pkcs8(pkcs8: &[u8]) -> Result<Vec<u8>, SigningError> {
    let key_pair =
        Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| SigningError::InvalidPrivateKey)?;
    Ok(key_pair.public_key().as_ref().to_vec())
}

/// Signs `statement` with a PKCS#8 Ed25519 private key, returning the 64-byte signature.
pub fn sign_statement(statement: &[u8], pkcs8: &[u8]) -> Result<Vec<u8>, SigningError> {
    let key_pair =
        Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| SigningError::InvalidPrivateKey)?;
    Ok(key_pair.sign(statement).as_ref().to_vec())
}

/// Verifies a detached Ed25519 `signature` over `statement` against a raw 32-byte `public_key`.
///
/// Returns [`SigningError::VerificationFailed`] for any mismatch — wrong key, malformed signature,
/// or a statement that differs by even one byte from what was signed.
pub fn verify_statement(
    statement: &[u8],
    signature: &[u8],
    public_key: &[u8],
) -> Result<(), SigningError> {
    if public_key.len() != ED25519_PUBLIC_KEY_LEN {
        return Err(SigningError::InvalidPublicKey(public_key.len()));
    }
    UnparsedPublicKey::new(&signature::ED25519, public_key)
        .verify(statement, signature)
        .map_err(|_| SigningError::VerificationFailed)
}

/// Computes the value recorded in `manifest.signatures[].key_id`: lowercase hex of the SHA-256
/// digest of the raw public key. Lets a verifier match a signature entry to a trusted key.
#[must_use]
pub fn key_id(public_key: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, public_key);
    hex::encode(digest.as_ref())
}

/// Parses a raw 32-byte Ed25519 public key from a hex string (surrounding whitespace ignored).
///
/// Convenience for CLI `--verify-key` inputs: a verifier holds the signer's public key as 64 hex
/// characters (the same form Akmon prints when signing). Returns [`SigningError::MalformedPublicKeyHex`]
/// for non-hex input and [`SigningError::InvalidPublicKey`] when the decoded length is not 32.
pub fn parse_public_key_hex(s: &str) -> Result<Vec<u8>, SigningError> {
    let bytes = hex::decode(s.trim()).map_err(|_| SigningError::MalformedPublicKeyHex)?;
    if bytes.len() != ED25519_PUBLIC_KEY_LEN {
        return Err(SigningError::InvalidPublicKey(bytes.len()));
    }
    Ok(bytes)
}

/// Outcome of checking one `manifest.signatures[]` entry against a set of trusted keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureOutcome {
    /// A trusted key validates the signature over the reconstructed statement.
    Verified,
    /// The entry names a trusted key (by `key_id`) but the signature does not validate — a tampered
    /// statement or a corrupt signature.
    Invalid,
    /// No trusted key validates the signature and none matches the entry's `key_id`.
    UnverifiedNoKey,
    /// The entry's `scheme` is not understood (only `ed25519` is defined in AGEF v0.1.2).
    UnsupportedScheme,
    /// The entry's `statement_version` or signature hex is malformed.
    Malformed,
}

/// Result of checking one signature entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureCheck {
    /// `key_id` copied from the manifest entry.
    pub key_id: String,
    /// `scheme` copied from the manifest entry.
    pub scheme: String,
    /// Verification outcome.
    pub outcome: SignatureOutcome,
}

/// Report from verifying all of a manifest's signatures against a set of trusted public keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignatureVerificationReport {
    /// Per-entry results, in manifest order.
    pub checks: Vec<SignatureCheck>,
}

impl SignatureVerificationReport {
    /// True when the manifest carried no signatures at all.
    #[must_use]
    pub fn is_unsigned(&self) -> bool {
        self.checks.is_empty()
    }

    /// True when at least one entry verified against a trusted key.
    #[must_use]
    pub fn any_verified(&self) -> bool {
        self.checks
            .iter()
            .any(|c| c.outcome == SignatureOutcome::Verified)
    }

    /// True when any entry named a trusted key but failed verification (a hard failure).
    #[must_use]
    pub fn any_invalid(&self) -> bool {
        self.checks
            .iter()
            .any(|c| c.outcome == SignatureOutcome::Invalid)
    }
}

/// Verifies every `manifest.signatures[]` entry against the supplied trusted Ed25519 public keys
/// (raw 32-byte each).
///
/// The `AGEF-SIG-v1` statement is reconstructed from the manifest's own `agef_version`,
/// `hash_algorithm`, `session.id`, and `session.head`, so a signature validates only if it covers
/// exactly this session. Independent of bundle integrity (decision D-18, S6): run integrity
/// verification first — this answers "who attested to this head", not "is the bundle consistent".
#[must_use]
pub fn verify_manifest_signatures(
    manifest: &crate::manifest::Manifest,
    trusted_keys: &[Vec<u8>],
) -> SignatureVerificationReport {
    let Some(signatures) = &manifest.signatures else {
        return SignatureVerificationReport::default();
    };
    let statement = signing_statement(
        &manifest.agef_version,
        &manifest.hash_algorithm,
        &manifest.session.id,
        &manifest.session.head,
    );
    let checks = signatures
        .iter()
        .map(|sig| check_signature_entry(sig, statement.as_bytes(), trusted_keys))
        .collect();
    SignatureVerificationReport { checks }
}

/// Checks one signature entry, trying every trusted key (key_id is a hint, not the trust anchor).
fn check_signature_entry(
    sig: &crate::manifest::ManifestSignature,
    statement: &[u8],
    trusted_keys: &[Vec<u8>],
) -> SignatureCheck {
    let with = |outcome| SignatureCheck {
        key_id: sig.key_id.clone(),
        scheme: sig.scheme.clone(),
        outcome,
    };
    if sig.scheme != SCHEME_ED25519 {
        return with(SignatureOutcome::UnsupportedScheme);
    }
    if sig.statement_version != SIG_STATEMENT_VERSION {
        return with(SignatureOutcome::Malformed);
    }
    let Ok(sig_bytes) = hex::decode(&sig.signature) else {
        return with(SignatureOutcome::Malformed);
    };
    if trusted_keys
        .iter()
        .any(|k| verify_statement(statement, &sig_bytes, k).is_ok())
    {
        return with(SignatureOutcome::Verified);
    }
    if trusted_keys.iter().any(|k| key_id(k) == sig.key_id) {
        return with(SignatureOutcome::Invalid);
    }
    with(SignatureOutcome::UnverifiedNoKey)
}

// ----------------------------------------------------------------------------------------------
// Operator-identity attestations (decision D-20; AGEF v0.1.3 §A.15).
//
// Purely additive over the AGEF-SIG-v1 head-signing scheme above: a separate, domain-separated
// statement (`AGEF-OPERATOR-v1`) binds a named human/role to the SAME session head, so a head
// signature can never be confused with an operator attestation and vice versa.
// ----------------------------------------------------------------------------------------------

/// Version tag of the canonical operator-identity statement (decision D-20; AGEF v0.1.3 §A.15).
pub const OPERATOR_STATEMENT_VERSION: &str = "AGEF-OPERATOR-v1";

/// The four operator identity fields bound by an `AGEF-OPERATOR-v1` attestation.
///
/// Every field is part of the signed statement; the manifest also records a `created_at` timestamp
/// that is metadata only and is NOT signed (see [`crate::manifest::OperatorAttestation`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorIdentity {
    /// Stable operator identifier (for example an email, employee id, or service account).
    pub operator_id: String,
    /// Human-readable display name of the operator.
    pub display_name: String,
    /// Role the operator acted in for this session (for example `release-engineer`).
    pub role: String,
    /// Organization the operator belongs to.
    pub org: String,
}

/// Builds the canonical `AGEF-OPERATOR-v1` statement that an operator attestation covers.
///
/// Fixed nine-line field order, LF line endings, a single trailing newline, and no other whitespace
/// (decision D-20; AGEF v0.1.3 §A.15). The first four lines mirror [`signing_statement`] so an
/// attestation binds to exactly one session; the last four lines carry the operator identity.
/// Callers pass the manifest's own `agef_version`, `hash_algorithm`, hyphenated `session_id`, and
/// lowercase-hex `head` verbatim, plus the operator identity fields.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn operator_statement(
    agef_version: &str,
    hash_algorithm: &str,
    session_id: &str,
    head_hex: &str,
    operator_id: &str,
    display_name: &str,
    role: &str,
    org: &str,
) -> String {
    format!(
        "{OPERATOR_STATEMENT_VERSION}\n\
         agef_version:{agef_version}\n\
         hash_algorithm:{hash_algorithm}\n\
         session_id:{session_id}\n\
         head:{head_hex}\n\
         operator_id:{operator_id}\n\
         display_name:{display_name}\n\
         role:{role}\n\
         org:{org}\n"
    )
}

/// Validates a single operator identity field for the line-oriented statement framing.
///
/// Rejects any value containing a `\n` or `\r`, which would otherwise inject extra lines into (or
/// truncate) the `AGEF-OPERATOR-v1` statement and let two distinct identities collapse to the same
/// signed bytes (decision D-20; an injection guard). Returns
/// [`SigningError::IllegalOperatorField`] on rejection.
pub fn validate_operator_field(value: &str) -> Result<(), SigningError> {
    if value.contains('\n') || value.contains('\r') {
        return Err(SigningError::IllegalOperatorField);
    }
    Ok(())
}

/// Builds a signed [`OperatorAttestation`](crate::manifest::OperatorAttestation) binding `identity`
/// to the manifest's session head (decision D-20; AGEF v0.1.3 §A.15).
///
/// Reconstructs the `AGEF-OPERATOR-v1` statement from the manifest's own `agef_version`,
/// `hash_algorithm`, `session.id`, and `session.head` plus the four identity fields, signs it with
/// the PKCS#8 Ed25519 private key, and records the result. `created_at` is recorded verbatim as
/// unsigned metadata.
///
/// # Errors
///
/// Returns [`SigningError::IllegalOperatorField`] if `operator_id` is empty or if any of the four
/// identity fields contains a `\n`/`\r`, and [`SigningError::InvalidPrivateKey`] if `pkcs8` is not a
/// valid Ed25519 private key.
pub fn build_operator_attestation(
    manifest: &crate::manifest::Manifest,
    identity: &OperatorIdentity,
    pkcs8: &[u8],
    created_at: &str,
) -> Result<crate::manifest::OperatorAttestation, SigningError> {
    if identity.operator_id.is_empty() {
        return Err(SigningError::IllegalOperatorField);
    }
    validate_operator_field(&identity.operator_id)?;
    validate_operator_field(&identity.display_name)?;
    validate_operator_field(&identity.role)?;
    validate_operator_field(&identity.org)?;

    let public_key = public_key_from_pkcs8(pkcs8)?;
    let statement = operator_statement(
        &manifest.agef_version,
        &manifest.hash_algorithm,
        &manifest.session.id,
        &manifest.session.head,
        &identity.operator_id,
        &identity.display_name,
        &identity.role,
        &identity.org,
    );
    let signature = sign_statement(statement.as_bytes(), pkcs8)?;
    Ok(crate::manifest::OperatorAttestation {
        scheme: SCHEME_ED25519.to_owned(),
        key_id: key_id(&public_key),
        statement_version: OPERATOR_STATEMENT_VERSION.to_owned(),
        operator_id: identity.operator_id.clone(),
        display_name: identity.display_name.clone(),
        role: identity.role.clone(),
        org: identity.org.clone(),
        signature: hex::encode(&signature),
        created_at: created_at.to_owned(),
    })
}

/// Outcome of checking one `manifest.operator_attestations[]` entry against a set of trusted keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorOutcome {
    /// A trusted key validates the attestation over the reconstructed `AGEF-OPERATOR-v1` statement.
    Verified,
    /// The entry names a trusted key (by `key_id`) but the signature does not validate — a tampered
    /// identity, a tampered head, or a corrupt signature.
    Invalid,
    /// No trusted key validates the attestation and none matches the entry's `key_id`.
    UnverifiedNoKey,
    /// The entry's `scheme` is not understood (only `ed25519` is defined in AGEF v0.1.3).
    UnsupportedScheme,
    /// The entry's `statement_version`, signature hex, or an identity field is malformed (an
    /// identity field containing `\n`/`\r` is rejected before any verification — a domain-separation
    /// and injection guard).
    Malformed,
}

/// Result of checking one operator-attestation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorCheck {
    /// `key_id` copied from the manifest entry.
    pub key_id: String,
    /// `scheme` copied from the manifest entry.
    pub scheme: String,
    /// `operator_id` copied from the manifest entry.
    pub operator_id: String,
    /// `display_name` copied from the manifest entry.
    pub display_name: String,
    /// `role` copied from the manifest entry.
    pub role: String,
    /// `org` copied from the manifest entry.
    pub org: String,
    /// `created_at` copied from the manifest entry.
    pub created_at: String,
    /// Verification outcome.
    pub outcome: OperatorOutcome,
}

/// Report from verifying all of a manifest's operator attestations against trusted public keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorVerificationReport {
    /// Per-entry results, in manifest order.
    pub checks: Vec<OperatorCheck>,
}

impl OperatorVerificationReport {
    /// True when the manifest carried no operator attestations at all.
    #[must_use]
    pub fn is_unattributed(&self) -> bool {
        self.checks.is_empty()
    }

    /// True when at least one entry verified against a trusted key.
    #[must_use]
    pub fn any_verified(&self) -> bool {
        self.checks
            .iter()
            .any(|c| c.outcome == OperatorOutcome::Verified)
    }

    /// True when any entry named a trusted key but failed verification (a hard failure).
    #[must_use]
    pub fn any_invalid(&self) -> bool {
        self.checks
            .iter()
            .any(|c| c.outcome == OperatorOutcome::Invalid)
    }
}

/// Verifies every `manifest.operator_attestations[]` entry against the supplied trusted Ed25519
/// public keys (raw 32-byte each) (decision D-20; AGEF v0.1.3 §A.15).
///
/// The `AGEF-OPERATOR-v1` statement is reconstructed from the manifest's own `agef_version`,
/// `hash_algorithm`, `session.id`, and `session.head`, plus the identity fields stored in the
/// entry, so an attestation validates only if it covers exactly this session AND the recorded
/// identity. `key_id` is a hint for matching, never the trust anchor. Independent of bundle
/// integrity and of head signatures: this answers "who claims to have operated this session".
#[must_use]
pub fn verify_operator_attestations(
    manifest: &crate::manifest::Manifest,
    trusted_operator_keys: &[Vec<u8>],
) -> OperatorVerificationReport {
    let Some(attestations) = &manifest.operator_attestations else {
        return OperatorVerificationReport::default();
    };
    let checks = attestations
        .iter()
        .map(|att| check_operator_entry(att, manifest, trusted_operator_keys))
        .collect();
    OperatorVerificationReport { checks }
}

/// Checks one operator-attestation entry, trying every trusted key (key_id is a hint, not the
/// trust anchor).
fn check_operator_entry(
    att: &crate::manifest::OperatorAttestation,
    manifest: &crate::manifest::Manifest,
    trusted_keys: &[Vec<u8>],
) -> OperatorCheck {
    let with = |outcome| OperatorCheck {
        key_id: att.key_id.clone(),
        scheme: att.scheme.clone(),
        operator_id: att.operator_id.clone(),
        display_name: att.display_name.clone(),
        role: att.role.clone(),
        org: att.org.clone(),
        created_at: att.created_at.clone(),
        outcome,
    };
    if att.scheme != SCHEME_ED25519 {
        return with(OperatorOutcome::UnsupportedScheme);
    }
    if att.statement_version != OPERATOR_STATEMENT_VERSION {
        return with(OperatorOutcome::Malformed);
    }
    // Reject any identity field with embedded newlines BEFORE verifying: such bytes can never have
    // been produced by a well-formed signer and would let two identities share signed bytes.
    if validate_operator_field(&att.operator_id).is_err()
        || validate_operator_field(&att.display_name).is_err()
        || validate_operator_field(&att.role).is_err()
        || validate_operator_field(&att.org).is_err()
    {
        return with(OperatorOutcome::Malformed);
    }
    let Ok(sig_bytes) = hex::decode(&att.signature) else {
        return with(OperatorOutcome::Malformed);
    };
    let statement = operator_statement(
        &manifest.agef_version,
        &manifest.hash_algorithm,
        &manifest.session.id,
        &manifest.session.head,
        &att.operator_id,
        &att.display_name,
        &att.role,
        &att.org,
    );
    if trusted_keys
        .iter()
        .any(|k| verify_statement(statement.as_bytes(), &sig_bytes, k).is_ok())
    {
        return with(OperatorOutcome::Verified);
    }
    if trusted_keys.iter().any(|k| key_id(k) == att.key_id) {
        return with(OperatorOutcome::Invalid);
    }
    with(OperatorOutcome::UnverifiedNoKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement() -> String {
        signing_statement(
            "0.1.2",
            "sha256",
            "550e8400-e29b-41d4-a716-446655440000",
            "ab12cd",
        )
    }

    #[test]
    fn statement_is_canonical() {
        assert_eq!(
            statement(),
            "AGEF-SIG-v1\nagef_version:0.1.2\nhash_algorithm:sha256\n\
             session_id:550e8400-e29b-41d4-a716-446655440000\nhead:ab12cd\n"
        );
    }

    #[test]
    fn sign_then_verify_roundtrips() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let stmt = statement();
        let sig = sign_statement(stmt.as_bytes(), &pkcs8).expect("sign");
        assert_eq!(sig.len(), 64);
        verify_statement(stmt.as_bytes(), &sig, &pubkey).expect("verify ok");
    }

    #[test]
    fn tampered_statement_fails_verification() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let stmt = statement();
        let sig = sign_statement(stmt.as_bytes(), &pkcs8).expect("sign");
        let mut tampered = stmt.into_bytes();
        let last = tampered.last_mut().expect("non-empty statement");
        *last ^= 0x01;
        assert!(matches!(
            verify_statement(&tampered, &sig, &pubkey),
            Err(SigningError::VerificationFailed)
        ));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let stmt = statement();
        let pkcs8_a = generate_pkcs8().expect("keygen a");
        let sig = sign_statement(stmt.as_bytes(), &pkcs8_a).expect("sign");
        let pkcs8_b = generate_pkcs8().expect("keygen b");
        let pubkey_b = public_key_from_pkcs8(&pkcs8_b).expect("pubkey b");
        assert!(verify_statement(stmt.as_bytes(), &sig, &pubkey_b).is_err());
    }

    #[test]
    fn wrong_public_key_length_is_rejected() {
        let err = verify_statement(b"msg", &[0u8; 64], &[0u8; 16]).unwrap_err();
        assert!(matches!(err, SigningError::InvalidPublicKey(16)));
    }

    #[test]
    fn key_id_is_deterministic_hex_sha256() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let id = key_id(&pubkey);
        assert_eq!(id.len(), 64);
        assert_eq!(id, key_id(&pubkey));
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_public_key_hex_roundtrips_and_rejects_bad_input() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let hexed = hex::encode(&pubkey);
        assert_eq!(parse_public_key_hex(&hexed).expect("parse"), pubkey);
        assert_eq!(
            parse_public_key_hex(&format!("  {hexed}\n")).expect("trim"),
            pubkey
        );
        assert!(matches!(
            parse_public_key_hex("nothex!!"),
            Err(SigningError::MalformedPublicKeyHex)
        ));
        assert!(matches!(
            parse_public_key_hex("ab12"),
            Err(SigningError::InvalidPublicKey(2))
        ));
    }

    fn signed_manifest(pkcs8: &[u8], head: &str) -> crate::manifest::Manifest {
        let pubkey = public_key_from_pkcs8(pkcs8).expect("pubkey");
        let stmt = signing_statement(
            "0.1.2",
            "sha256",
            "550e8400-e29b-41d4-a716-446655440000",
            head,
        );
        let sig = sign_statement(stmt.as_bytes(), pkcs8).expect("sign");
        crate::manifest::Manifest {
            agef_version: "0.1.2".to_owned(),
            producer: crate::manifest::Producer {
                name: "akmon".to_owned(),
                version: "t".to_owned(),
            },
            session: crate::manifest::SessionMetadata {
                id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                head: head.to_owned(),
                created_at: "2026-05-04T14:00:00Z".to_owned(),
                ended_at: "2026-05-04T14:01:00Z".to_owned(),
            },
            hash_algorithm: "sha256".to_owned(),
            object_count: 1,
            event_count: 2,
            signatures: Some(vec![crate::manifest::ManifestSignature {
                scheme: SCHEME_ED25519.to_owned(),
                key_id: key_id(&pubkey),
                statement_version: SIG_STATEMENT_VERSION.to_owned(),
                signature: hex::encode(&sig),
                created_at: "2026-05-04T14:01:00Z".to_owned(),
            }]),
            operator_attestations: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn unsigned_manifest_reports_unsigned() {
        let mut m = signed_manifest(&generate_pkcs8().expect("keygen"), "deadbeef");
        m.signatures = None;
        let report = verify_manifest_signatures(&m, &[]);
        assert!(report.is_unsigned());
        assert!(!report.any_verified());
    }

    #[test]
    fn valid_signature_verifies_with_trusted_key() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let m = signed_manifest(&pkcs8, "deadbeef");
        let report = verify_manifest_signatures(&m, &[pubkey]);
        assert!(report.any_verified());
        assert!(!report.any_invalid());
        assert_eq!(report.checks[0].outcome, SignatureOutcome::Verified);
    }

    #[test]
    fn signature_without_trusted_key_is_unverified() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let m = signed_manifest(&pkcs8, "deadbeef");
        let report = verify_manifest_signatures(&m, &[]);
        assert!(!report.is_unsigned());
        assert!(!report.any_verified());
        assert_eq!(report.checks[0].outcome, SignatureOutcome::UnverifiedNoKey);
    }

    #[test]
    fn tampered_head_makes_matching_key_invalid() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let mut m = signed_manifest(&pkcs8, "deadbeef");
        m.session.head = "feedface".to_owned();
        let report = verify_manifest_signatures(&m, &[pubkey]);
        assert!(report.any_invalid());
        assert_eq!(report.checks[0].outcome, SignatureOutcome::Invalid);
    }

    #[test]
    fn unsupported_scheme_is_flagged() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let mut m = signed_manifest(&pkcs8, "deadbeef");
        m.signatures.as_mut().expect("sigs")[0].scheme = "rsa".to_owned();
        let report = verify_manifest_signatures(&m, &[]);
        assert_eq!(
            report.checks[0].outcome,
            SignatureOutcome::UnsupportedScheme
        );
    }

    #[test]
    fn malformed_signature_hex_is_flagged() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let mut m = signed_manifest(&pkcs8, "deadbeef");
        m.signatures.as_mut().expect("sigs")[0].signature = "nothex!!".to_owned();
        let report = verify_manifest_signatures(&m, &[]);
        assert_eq!(report.checks[0].outcome, SignatureOutcome::Malformed);
    }

    // --- Operator-identity attestations (decision D-20; AGEF v0.1.3 §A.15) ----------------------

    fn sample_identity() -> OperatorIdentity {
        OperatorIdentity {
            operator_id: "alice@example.com".to_owned(),
            display_name: "Alice Example".to_owned(),
            role: "release-engineer".to_owned(),
            org: "Acme".to_owned(),
        }
    }

    /// Builds an unsigned base manifest with no attestations, at the current head.
    fn base_manifest(head: &str) -> crate::manifest::Manifest {
        let mut m = signed_manifest(&generate_pkcs8().expect("keygen"), head);
        m.signatures = None;
        m
    }

    /// Builds a manifest carrying a single operator attestation from `pkcs8`/`identity` over `head`.
    fn attested_manifest(
        pkcs8: &[u8],
        identity: &OperatorIdentity,
        head: &str,
    ) -> crate::manifest::Manifest {
        let mut m = base_manifest(head);
        let att = build_operator_attestation(&m, identity, pkcs8, "2026-06-06T00:00:00Z")
            .expect("build attestation");
        m.operator_attestations = Some(vec![att]);
        m
    }

    #[test]
    fn operator_statement_is_canonical() {
        let stmt = operator_statement(
            "0.1.3",
            "sha256",
            "550e8400-e29b-41d4-a716-446655440000",
            "ab12cd",
            "alice@example.com",
            "Alice Example",
            "release-engineer",
            "Acme",
        );
        assert_eq!(
            stmt,
            "AGEF-OPERATOR-v1\nagef_version:0.1.3\nhash_algorithm:sha256\n\
             session_id:550e8400-e29b-41d4-a716-446655440000\nhead:ab12cd\n\
             operator_id:alice@example.com\ndisplay_name:Alice Example\n\
             role:release-engineer\norg:Acme\n"
        );
    }

    #[test]
    fn build_then_verify_roundtrips() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        let report = verify_operator_attestations(&m, &[pubkey]);
        assert!(report.any_verified());
        assert!(!report.any_invalid());
        assert_eq!(report.checks[0].outcome, OperatorOutcome::Verified);
        assert_eq!(report.checks[0].operator_id, "alice@example.com");
        assert_eq!(report.checks[0].role, "release-engineer");
    }

    #[test]
    fn tampered_identity_field_makes_matching_key_invalid() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let mut m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        // Flip a byte of the stored display_name; the key still matches by key_id.
        m.operator_attestations.as_mut().expect("atts")[0].display_name =
            "Alice Examplf".to_owned();
        let report = verify_operator_attestations(&m, &[pubkey]);
        assert!(report.any_invalid());
        assert_eq!(report.checks[0].outcome, OperatorOutcome::Invalid);
    }

    #[test]
    fn tampered_head_makes_attestation_invalid() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let mut m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        m.session.head = "feedface".to_owned();
        let report = verify_operator_attestations(&m, &[pubkey]);
        assert!(report.any_invalid());
        assert_eq!(report.checks[0].outcome, OperatorOutcome::Invalid);
    }

    #[test]
    fn attestation_without_trusted_key_is_unverified() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        let report = verify_operator_attestations(&m, &[]);
        assert!(!report.is_unattributed());
        assert!(!report.any_verified());
        assert_eq!(report.checks[0].outcome, OperatorOutcome::UnverifiedNoKey);
    }

    #[test]
    fn unattributed_manifest_reports_unattributed() {
        let m = base_manifest("deadbeef");
        let report = verify_operator_attestations(&m, &[]);
        assert!(report.is_unattributed());
        assert!(!report.any_verified());
    }

    #[test]
    fn newline_in_field_rejected_at_build() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let m = base_manifest("deadbeef");
        let mut id = sample_identity();
        id.display_name = "Alice\nExample".to_owned();
        let err = build_operator_attestation(&m, &id, &pkcs8, "2026-06-06T00:00:00Z").unwrap_err();
        assert!(matches!(err, SigningError::IllegalOperatorField));
        // A carriage return is rejected too.
        let mut id_cr = sample_identity();
        id_cr.org = "Ac\rme".to_owned();
        assert!(matches!(
            build_operator_attestation(&m, &id_cr, &pkcs8, "2026-06-06T00:00:00Z"),
            Err(SigningError::IllegalOperatorField)
        ));
    }

    #[test]
    fn newline_in_field_flagged_malformed_at_verify() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let mut m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        // Inject a newline into a stored field after the fact; verification must reject it before
        // attempting any signature check (domain-separation + injection guard).
        m.operator_attestations.as_mut().expect("atts")[0].role = "release\nengineer".to_owned();
        let report = verify_operator_attestations(&m, &[pubkey]);
        assert_eq!(report.checks[0].outcome, OperatorOutcome::Malformed);
    }

    #[test]
    fn empty_operator_id_rejected_at_build() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let m = base_manifest("deadbeef");
        let mut id = sample_identity();
        id.operator_id = String::new();
        let err = build_operator_attestation(&m, &id, &pkcs8, "2026-06-06T00:00:00Z").unwrap_err();
        assert!(matches!(err, SigningError::IllegalOperatorField));
    }

    #[test]
    fn unsupported_scheme_flagged() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let mut m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        m.operator_attestations.as_mut().expect("atts")[0].scheme = "rsa".to_owned();
        let report = verify_operator_attestations(&m, &[]);
        assert_eq!(report.checks[0].outcome, OperatorOutcome::UnsupportedScheme);
    }

    #[test]
    fn malformed_signature_hex_flagged() {
        let pkcs8 = generate_pkcs8().expect("keygen");
        let mut m = attested_manifest(&pkcs8, &sample_identity(), "deadbeef");
        m.operator_attestations.as_mut().expect("atts")[0].signature = "nothex!!".to_owned();
        let report = verify_operator_attestations(&m, &[]);
        assert_eq!(report.checks[0].outcome, OperatorOutcome::Malformed);
    }

    #[test]
    fn cross_statement_non_confusion() {
        // An AGEF-SIG-v1 head signature over the SAME session head must NOT verify when placed in
        // an OperatorAttestation and checked as AGEF-OPERATOR-v1 — proves domain separation.
        let pkcs8 = generate_pkcs8().expect("keygen");
        let pubkey = public_key_from_pkcs8(&pkcs8).expect("pubkey");
        let id = sample_identity();
        let mut m = base_manifest("deadbeef");
        // Sign the HEAD statement (AGEF-SIG-v1), not the operator statement.
        let head_stmt = signing_statement(
            &m.agef_version,
            &m.hash_algorithm,
            &m.session.id,
            &m.session.head,
        );
        let head_sig = sign_statement(head_stmt.as_bytes(), &pkcs8).expect("sign head");
        m.operator_attestations = Some(vec![crate::manifest::OperatorAttestation {
            scheme: SCHEME_ED25519.to_owned(),
            key_id: key_id(&pubkey),
            statement_version: OPERATOR_STATEMENT_VERSION.to_owned(),
            operator_id: id.operator_id.clone(),
            display_name: id.display_name.clone(),
            role: id.role.clone(),
            org: id.org.clone(),
            signature: hex::encode(&head_sig),
            created_at: "2026-06-06T00:00:00Z".to_owned(),
        }]);
        let report = verify_operator_attestations(&m, &[pubkey]);
        // The trusted key matches by key_id, but the head signature does not cover the operator
        // statement, so the outcome is Invalid (NOT Verified).
        assert_ne!(report.checks[0].outcome, OperatorOutcome::Verified);
        assert_eq!(report.checks[0].outcome, OperatorOutcome::Invalid);
    }
}
