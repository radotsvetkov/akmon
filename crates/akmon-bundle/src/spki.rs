//! Ed25519 `SubjectPublicKeyInfo` (SPKI) DER/PEM encoding for offline verification.
//!
//! Akmon's competitive wedge is that its tamper-evident evidence can be verified offline with
//! standard tools — no Akmon binary, no cloud. Stock `openssl` cannot ingest a bare 32-byte
//! Ed25519 public key; it requires the SPKI DER/PEM wrapper (RFC 8410). These pure, deterministic
//! helpers re-encode the SAME raw public key already recorded in `manifest.signatures[]` into the
//! SPKI form `openssl pkeyutl -verify -pubin` expects.
//!
//! This is an encoding of an EXISTING key, not a new key, scheme, or on-disk format — it adds no
//! substrate. The DER is pure byte concatenation; the PEM uses a tiny standard-alphabet base64
//! encoder so the crate gains no new dependency.

use crate::signing::SigningError;

/// Length in bytes of an Ed25519 `SubjectPublicKeyInfo` DER encoding.
pub const ED25519_SPKI_DER_LEN: usize = 44;

/// Length in bytes of a raw Ed25519 public key.
const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Fixed 12-byte ASN.1 SPKI prefix for an Ed25519 public key (RFC 8410 §4):
/// `SEQUENCE { SEQUENCE { OID 1.3.101.112 } BIT STRING (0 unused bits) }`.
///
/// Verified byte-identical to the first 12 bytes of `openssl pkey -pubout -outform DER`.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Wraps a raw 32-byte Ed25519 public key into its 44-byte SPKI DER encoding.
///
/// Returns [`SigningError::InvalidPublicKey`] with the actual length when
/// `public_key.len() != 32`.
pub fn ed25519_spki_der(public_key: &[u8]) -> Result<[u8; ED25519_SPKI_DER_LEN], SigningError> {
    if public_key.len() != ED25519_PUBLIC_KEY_LEN {
        return Err(SigningError::InvalidPublicKey(public_key.len()));
    }
    let mut der = [0u8; ED25519_SPKI_DER_LEN];
    der[..ED25519_SPKI_PREFIX.len()].copy_from_slice(&ED25519_SPKI_PREFIX);
    der[ED25519_SPKI_PREFIX.len()..].copy_from_slice(public_key);
    Ok(der)
}

/// Encodes a raw 32-byte Ed25519 public key as a PEM `PUBLIC KEY` block:
/// `"-----BEGIN PUBLIC KEY-----\n" + base64(spki_der) + "\n-----END PUBLIC KEY-----\n"`.
///
/// The 44-byte DER base64s to exactly 60 characters, so no internal line wrap is needed and
/// `openssl` accepts it. Returns [`SigningError::InvalidPublicKey`] when `public_key.len() != 32`.
pub fn ed25519_spki_pem(public_key: &[u8]) -> Result<String, SigningError> {
    let der = ed25519_spki_der(public_key)?;
    Ok(format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        base64_encode(&der)
    ))
}

/// Standard-alphabet (RFC 4648 §4) base64 encoder with `=` padding.
///
/// Tiny and dependency-free: the SPKI PEM wrapper is the only caller and it always encodes a
/// fixed 44-byte input. Locked against the RFC 4648 test vectors in the unit tests below.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spki_der_has_fixed_prefix_and_key() {
        let key = [0x01u8; 32];
        let der = ed25519_spki_der(&key).expect("der");
        assert_eq!(der.len(), 44);
        assert_eq!(&der[..12], &ED25519_SPKI_PREFIX);
        assert_eq!(&der[12..], &key);
    }

    #[test]
    fn spki_der_rejects_wrong_length() {
        let err = ed25519_spki_der(&[0u8; 16]).unwrap_err();
        assert!(matches!(err, SigningError::InvalidPublicKey(16)));
    }

    #[test]
    fn pem_round_trips() {
        let key = [0x01u8; 32];
        let pem = ed25519_spki_pem(&key).expect("pem");
        // Golden string verified byte-identical to `openssl pkey -pubout` for key=[0x01; 32].
        assert_eq!(
            pem,
            "-----BEGIN PUBLIC KEY-----\n\
             MCowBQYDK2VwAyEAAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\n\
             -----END PUBLIC KEY-----\n"
        );
    }

    #[test]
    fn base64_encode_known_vectors() {
        // RFC 4648 §10 test vectors.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    }
}
