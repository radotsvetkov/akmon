//! Hash primitives for journal object and event addressing.

use crate::error::{JournalError, Result};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Length in bytes of a SHA-256 digest.
pub const SHA256_LEN: usize = 32;
/// Length in bytes of a BLAKE3 digest.
pub const BLAKE3_LEN: usize = 32;

/// Configurable hash algorithms supported by the journal substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashAlgorithm {
    /// SHA-256 (default in AGEF v0.1).
    Sha256,
    /// BLAKE3 (optional in AGEF v0.1).
    Blake3,
}

impl std::fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Sha256 => "sha256",
            Self::Blake3 => "blake3",
        };
        f.write_str(value)
    }
}

/// On-the-wire hash representation for CBOR-encoded events.
///
/// This carries only the digest bytes; hash algorithm comes from journal metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireHash(pub [u8; 32]);

impl Serialize for WireHash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for WireHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = <Vec<u8>>::deserialize(deserializer)?;
        if raw.len() != SHA256_LEN {
            return Err(D::Error::custom(format!(
                "expected 32-byte hash, got {} bytes",
                raw.len()
            )));
        }
        let mut bytes = [0_u8; SHA256_LEN];
        bytes.copy_from_slice(&raw);
        Ok(Self(bytes))
    }
}

/// A 32-byte digest plus its associated algorithm.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash {
    /// Hash algorithm used to compute this digest.
    pub algorithm: HashAlgorithm,
    /// Raw digest bytes.
    pub bytes: [u8; 32],
}

impl std::fmt::Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Hash {
    /// Creates a hash from explicit algorithm and raw bytes.
    pub fn from_bytes(algorithm: HashAlgorithm, bytes: [u8; 32]) -> Self {
        Self { algorithm, bytes }
    }

    /// Parses a hex string into a typed hash.
    pub fn parse_hex(algorithm: HashAlgorithm, hex_text: &str) -> Result<Self> {
        let decoded = hex::decode(hex_text)
            .map_err(|err| JournalError::HashParse(format!("invalid hex string: {err}")))?;
        if decoded.len() != SHA256_LEN {
            return Err(JournalError::HashParse(format!(
                "expected {SHA256_LEN} bytes, got {}",
                decoded.len()
            )));
        }
        let mut bytes = [0_u8; SHA256_LEN];
        bytes.copy_from_slice(&decoded);
        Ok(Self { algorithm, bytes })
    }

    /// Formats this hash as lowercase hexadecimal.
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }

    /// Converts runtime hash into wire representation.
    pub fn to_wire(&self) -> WireHash {
        WireHash(self.bytes)
    }

    /// Converts wire representation into runtime hash using explicit algorithm context.
    pub fn from_wire(algorithm: HashAlgorithm, wire: WireHash) -> Self {
        Self {
            algorithm,
            bytes: wire.0,
        }
    }
}

/// Hashes raw bytes using the selected algorithm.
pub fn digest_bytes(algorithm: HashAlgorithm, input: &[u8]) -> Hash {
    let bytes = match algorithm {
        HashAlgorithm::Sha256 => {
            use sha2::Digest as _;
            let digest = sha2::Sha256::digest(input);
            let mut out = [0_u8; SHA256_LEN];
            out.copy_from_slice(&digest);
            out
        }
        HashAlgorithm::Blake3 => blake3::hash(input).into(),
    };
    Hash { algorithm, bytes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_sha256_and_blake3() {
        let input = b"akmon-layer-2-hex-roundtrip";
        let sha = digest_bytes(HashAlgorithm::Sha256, input);
        let blake = digest_bytes(HashAlgorithm::Blake3, input);

        let parsed_sha = Hash::parse_hex(HashAlgorithm::Sha256, &sha.to_hex());
        assert!(parsed_sha.is_ok());
        assert_eq!(parsed_sha.unwrap_or_else(|_| unreachable!()), sha);

        let parsed_blake = Hash::parse_hex(HashAlgorithm::Blake3, &blake.to_hex());
        assert!(parsed_blake.is_ok());
        assert_eq!(parsed_blake.unwrap_or_else(|_| unreachable!()), blake);
    }

    #[test]
    fn digest_bytes_is_deterministic_and_algorithm_distinct() {
        let a1 = digest_bytes(HashAlgorithm::Sha256, b"same-input");
        let a2 = digest_bytes(HashAlgorithm::Sha256, b"same-input");
        let b = digest_bytes(HashAlgorithm::Sha256, b"different-input");
        let c = digest_bytes(HashAlgorithm::Blake3, b"same-input");

        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert_ne!(a1.algorithm, c.algorithm);
        assert_ne!(a1.bytes, c.bytes);
    }

    #[test]
    fn cbor_wire_hash_encoding_is_byte_string_len_32() {
        let known = WireHash([0xAB; 32]);
        let mut encoded = Vec::new();
        let encoded_result = ciborium::ser::into_writer(&known, &mut encoded);
        assert!(encoded_result.is_ok());

        assert_eq!(encoded.len(), 34);
        assert_eq!(encoded[0], 0x58);
        assert_eq!(encoded[1], 0x20);
        assert_eq!(&encoded[2..], &known.0);
        assert_ne!(encoded[0], 0x98);
    }

    #[test]
    fn cbor_wire_hash_roundtrip_preserves_bytes() {
        let original = WireHash([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
            0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B,
            0x1C, 0x1D, 0x1E, 0x1F,
        ]);
        let mut encoded = Vec::new();
        let ser = ciborium::ser::into_writer(&original, &mut encoded);
        assert!(ser.is_ok());

        let decoded: std::result::Result<WireHash, _> =
            ciborium::de::from_reader(encoded.as_slice());
        assert!(decoded.is_ok());
        let decoded = decoded.unwrap_or_else(|_| unreachable!());
        assert_eq!(decoded, original);
    }

    #[test]
    fn parse_hex_rejects_non_32_byte_inputs() {
        let err = Hash::parse_hex(HashAlgorithm::Sha256, "00").err();
        assert!(err.is_some());
        match err.unwrap_or_else(|| unreachable!()) {
            JournalError::HashParse(_) => {}
            other => panic!("unexpected error variant: {other}"),
        }
    }

    #[test]
    fn hash_to_wire_from_wire_preserves_explicit_algorithm() {
        let original = Hash::from_bytes(HashAlgorithm::Blake3, [0x5A; 32]);
        let wire = original.to_wire();
        let restored = Hash::from_wire(HashAlgorithm::Blake3, wire);
        assert_eq!(restored.algorithm, HashAlgorithm::Blake3);
        assert_eq!(restored.bytes, [0x5A; 32]);
        assert_eq!(restored, original);
    }

    #[test]
    fn postcard_hash_roundtrip_preserves_algorithm_tag() {
        let sha = Hash::from_bytes(HashAlgorithm::Sha256, [0x11; 32]);
        let blake = Hash::from_bytes(HashAlgorithm::Blake3, [0x22; 32]);

        let sha_encoded = postcard::to_allocvec(&sha);
        assert!(sha_encoded.is_ok());
        let sha_encoded = sha_encoded.unwrap_or_else(|_| unreachable!());
        let sha_decoded: std::result::Result<Hash, _> = postcard::from_bytes(&sha_encoded);
        assert!(sha_decoded.is_ok());
        assert_eq!(
            sha_decoded.unwrap_or_else(|_| unreachable!()).algorithm,
            HashAlgorithm::Sha256
        );

        let blake_encoded = postcard::to_allocvec(&blake);
        assert!(blake_encoded.is_ok());
        let blake_encoded = blake_encoded.unwrap_or_else(|_| unreachable!());
        let blake_decoded: std::result::Result<Hash, _> = postcard::from_bytes(&blake_encoded);
        assert!(blake_decoded.is_ok());
        assert_eq!(
            blake_decoded.unwrap_or_else(|_| unreachable!()).algorithm,
            HashAlgorithm::Blake3
        );
    }
}
