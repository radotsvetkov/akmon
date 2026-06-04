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
    /// The system random number generator could not produce a key.
    #[error("Ed25519 key generation failed")]
    KeyGeneration,
    /// Verification failed: wrong key, malformed signature, or a tampered statement.
    #[error("Ed25519 signature verification failed")]
    VerificationFailed,
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
}
