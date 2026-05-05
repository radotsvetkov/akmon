//! Akmon-specific redaction sentinel primitives.

use akmon_journal::Hash;
use ciborium::Value;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Akmon-specific redaction sentinel payload stored as canonical CBOR object bytes.
///
/// Hash algorithm context for `original_hash` comes from the containing bundle's
/// `manifest.json` (`hash_algorithm` field), not from the sentinel itself.
/// Sentinels only exist within AGEF bundles; out-of-bundle inspection is not a
/// supported usage pattern.
///
/// If AGEF v0.2 standardizes redaction sentinels and requires self-description,
/// this struct will gain an `original_hash_algorithm` field at that time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentinelMarker {
    /// Marker bit indicating this object is a redaction sentinel.
    pub akmon_redacted: bool,
    /// Original object hash in lowercase hex.
    pub original_hash: String,
    /// Original object size in bytes.
    pub original_size: u64,
    /// Operator-provided redaction reason.
    pub reason: String,
    /// RFC3339 UTC timestamp for when redaction was applied.
    pub redacted_at: String,
}

/// Closed parse-error set for sentinel detection/decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentinelParseError {
    /// CBOR payload is malformed or non-canonical for sentinel candidates.
    InvalidCbor(String),
    /// Sentinel candidate structure is invalid (missing fields, wrong types, etc.).
    InvalidShape(String),
    /// `original_hash` is not a valid 32-byte lowercase/uppercase hex digest string.
    InvalidOriginalHash(String),
    /// `redacted_at` is not a valid RFC3339 timestamp.
    InvalidTimestamp(String),
}

impl std::fmt::Display for SentinelParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCbor(msg) => write!(f, "invalid cbor: {msg}"),
            Self::InvalidShape(msg) => write!(f, "invalid sentinel shape: {msg}"),
            Self::InvalidOriginalHash(msg) => write!(f, "invalid original_hash: {msg}"),
            Self::InvalidTimestamp(msg) => write!(f, "invalid redacted_at: {msg}"),
        }
    }
}

impl std::error::Error for SentinelParseError {}

/// Constructs a sentinel marker from original object metadata.
pub fn sentinel_from_original(
    original_hash: &Hash,
    original_size: u64,
    reason: &str,
    redacted_at: OffsetDateTime,
) -> SentinelMarker {
    // NOTE: Rfc3339 well-known formatter produces variable precision
    // (nanosecond when non-zero, second when zero). Acceptable for
    // v2.0.0 (Akmon is the sole producer). If future AGEF versions
    // standardize sentinel timestamps, pin to a specific precision
    // (likely millisecond) for cross-producer determinism.
    let redacted_at = match redacted_at.format(&Rfc3339) {
        Ok(ts) => ts,
        Err(_) => "1970-01-01T00:00:00Z".to_owned(),
    };
    SentinelMarker {
        akmon_redacted: true,
        original_hash: original_hash.to_hex(),
        original_size,
        reason: reason.to_owned(),
        redacted_at,
    }
}

/// Serializes a sentinel marker into canonical CBOR bytes.
pub fn sentinel_to_canonical_cbor(marker: &SentinelMarker) -> Result<Vec<u8>, SentinelParseError> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(marker, &mut out)
        .map_err(|err| SentinelParseError::InvalidCbor(err.to_string()))?;
    Ok(out)
}

/// Tries to parse bytes as an Akmon sentinel marker.
///
/// Returns `Ok(None)` when bytes are not a sentinel. Returns `Err(...)` when bytes
/// are sentinel candidates but malformed.
pub fn try_parse_sentinel(bytes: &[u8]) -> Result<Option<SentinelMarker>, SentinelParseError> {
    let value: Value = match ciborium::de::from_reader(bytes) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let map = match value {
        Value::Map(map) => map,
        _ => return Ok(None),
    };

    let has_marker_field = map
        .iter()
        .any(|(k, _)| matches!(k, Value::Text(key) if key == "akmon_redacted"));
    if !has_marker_field {
        return Ok(None);
    }

    let marker_value = map
        .iter()
        .find_map(|(k, v)| match k {
            Value::Text(key) if key == "akmon_redacted" => Some(v),
            _ => None,
        })
        .ok_or_else(|| {
            SentinelParseError::InvalidShape("missing akmon_redacted field".to_owned())
        })?;
    match marker_value {
        Value::Bool(true) => {}
        Value::Bool(false) => {
            return Err(SentinelParseError::InvalidShape(
                "akmon_redacted must be true".to_owned(),
            ));
        }
        _ => {
            return Err(SentinelParseError::InvalidShape(
                "akmon_redacted must be boolean".to_owned(),
            ));
        }
    }

    let canonical = {
        let mut out = Vec::new();
        ciborium::ser::into_writer(&Value::Map(map.clone()), &mut out)
            .map_err(|err| SentinelParseError::InvalidCbor(err.to_string()))?;
        out
    };
    if canonical != bytes {
        return Err(SentinelParseError::InvalidCbor(
            "non-canonical cbor encoding".to_owned(),
        ));
    }

    let marker: SentinelMarker = ciborium::de::from_reader(bytes)
        .map_err(|err| SentinelParseError::InvalidShape(err.to_string()))?;
    validate_sentinel_marker(&marker)?;
    Ok(Some(marker))
}

/// Returns true when bytes parse as a valid Akmon sentinel marker.
///
/// Malformed sentinel-shaped bytes (e.g., missing required fields, invalid
/// timestamps) return false. Use `try_parse_sentinel` for distinguishing
/// "not a sentinel" from "broken sentinel candidate."
pub fn is_sentinel(bytes: &[u8]) -> bool {
    matches!(try_parse_sentinel(bytes), Ok(Some(_)))
}

fn validate_sentinel_marker(marker: &SentinelMarker) -> Result<(), SentinelParseError> {
    if !marker.akmon_redacted {
        return Err(SentinelParseError::InvalidShape(
            "akmon_redacted must be true".to_owned(),
        ));
    }
    let original_hash = marker.original_hash.trim();
    let digest = hex::decode(original_hash)
        .map_err(|err| SentinelParseError::InvalidOriginalHash(err.to_string()))?;
    if digest.len() != 32 {
        return Err(SentinelParseError::InvalidOriginalHash(format!(
            "expected 32 bytes, found {}",
            digest.len()
        )));
    }
    OffsetDateTime::parse(&marker.redacted_at, &Rfc3339)
        .map_err(|err| SentinelParseError::InvalidTimestamp(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_journal::{HashAlgorithm, digest_bytes};

    fn sample_hash(seed: u8) -> Hash {
        Hash::from_bytes(HashAlgorithm::Sha256, [seed; 32])
    }

    #[test]
    fn t_sentinel_roundtrip_construct_serialize_parse() {
        let marker = sentinel_from_original(
            &sample_hash(0x2A),
            42,
            "contains secrets",
            OffsetDateTime::from_unix_timestamp(1_700_000_000)
                .unwrap_or(OffsetDateTime::UNIX_EPOCH),
        );
        let bytes = sentinel_to_canonical_cbor(&marker).unwrap_or_else(|_| unreachable!());
        let parsed = try_parse_sentinel(&bytes).unwrap_or_else(|_| unreachable!());
        assert_eq!(parsed, Some(marker));
    }

    #[test]
    fn t_sentinel_non_sentinel_bytes_returns_none() {
        let bytes = b"not cbor sentinel bytes";
        let parsed = try_parse_sentinel(bytes).unwrap_or_else(|_| unreachable!());
        assert!(parsed.is_none());
    }

    #[test]
    fn t_sentinel_malformed_missing_field_rejected() {
        let bytes = [
            0xA1, 0x6E, b'a', b'k', b'm', b'o', b'n', b'_', b'r', b'e', b'd', b'a', b'c', b't',
            b'e', b'd', 0xF5,
        ];
        let err = try_parse_sentinel(&bytes).expect_err("missing fields must fail");
        assert!(matches!(err, SentinelParseError::InvalidShape(_)));
    }

    #[test]
    fn t_sentinel_wrong_field_types_rejected() {
        let marker = SentinelMarker {
            akmon_redacted: true,
            original_hash: sample_hash(0x44).to_hex(),
            original_size: 9,
            reason: "x".to_owned(),
            redacted_at: "2024-01-01T00:00:00Z".to_owned(),
        };
        let mut value: Value = ciborium::de::from_reader(
            sentinel_to_canonical_cbor(&marker)
                .unwrap_or_else(|_| unreachable!())
                .as_slice(),
        )
        .unwrap_or_else(|_| unreachable!());
        if let Value::Map(ref mut entries) = value {
            for (k, v) in entries.iter_mut() {
                if matches!(k, Value::Text(key) if key == "original_size") {
                    *v = Value::Text("nine".to_owned());
                }
            }
        }
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).unwrap_or_else(|_| unreachable!());
        let err = try_parse_sentinel(&bytes).expect_err("wrong types must fail");
        assert!(matches!(err, SentinelParseError::InvalidShape(_)));
    }

    #[test]
    fn t_sentinel_invalid_hash_string_rejected() {
        let marker = SentinelMarker {
            akmon_redacted: true,
            original_hash: "gg".to_owned(),
            original_size: 1,
            reason: "x".to_owned(),
            redacted_at: "2024-01-01T00:00:00Z".to_owned(),
        };
        let bytes = sentinel_to_canonical_cbor(&marker).unwrap_or_else(|_| unreachable!());
        let err = try_parse_sentinel(&bytes).expect_err("invalid hash must fail");
        assert!(matches!(err, SentinelParseError::InvalidOriginalHash(_)));
    }

    #[test]
    fn t_sentinel_invalid_timestamp_rejected() {
        let marker = SentinelMarker {
            akmon_redacted: true,
            original_hash: sample_hash(0x55).to_hex(),
            original_size: 1,
            reason: "x".to_owned(),
            redacted_at: "not-a-time".to_owned(),
        };
        let bytes = sentinel_to_canonical_cbor(&marker).unwrap_or_else(|_| unreachable!());
        let err = try_parse_sentinel(&bytes).expect_err("invalid timestamp must fail");
        assert!(matches!(err, SentinelParseError::InvalidTimestamp(_)));
    }

    #[test]
    fn t_sentinel_canonical_enforcement_rejects_noncanonical_encoding() {
        let marker = sentinel_from_original(
            &sample_hash(0x77),
            7,
            "x",
            OffsetDateTime::from_unix_timestamp(1_700_000_000)
                .unwrap_or(OffsetDateTime::UNIX_EPOCH),
        );
        let canonical = sentinel_to_canonical_cbor(&marker).unwrap_or_else(|_| unreachable!());
        assert_eq!(canonical.first().copied(), Some(0xA5));
        let mut non_canonical = Vec::with_capacity(canonical.len() + 1);
        non_canonical.push(0xB8);
        non_canonical.push(0x05);
        non_canonical.extend_from_slice(&canonical[1..]);
        let err = try_parse_sentinel(&non_canonical).expect_err("non-canonical must fail");
        assert!(matches!(err, SentinelParseError::InvalidCbor(_)));
    }

    #[test]
    fn t_sentinel_deterministic_bytes_same_input_same_output() {
        let ts = OffsetDateTime::from_unix_timestamp(1_700_000_123)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let m1 = sentinel_from_original(&sample_hash(0x10), 99, "same", ts);
        let m2 = sentinel_from_original(&sample_hash(0x10), 99, "same", ts);
        let b1 = sentinel_to_canonical_cbor(&m1).unwrap_or_else(|_| unreachable!());
        let b2 = sentinel_to_canonical_cbor(&m2).unwrap_or_else(|_| unreachable!());
        assert_eq!(b1, b2);
    }

    #[test]
    fn t_sentinel_hash_determinism_same_bytes_same_digest() {
        let ts = OffsetDateTime::from_unix_timestamp(1_700_000_456)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let marker = sentinel_from_original(&sample_hash(0x99), 5, "reason", ts);
        let bytes = sentinel_to_canonical_cbor(&marker).unwrap_or_else(|_| unreachable!());
        let h1 = digest_bytes(HashAlgorithm::Sha256, &bytes);
        let h2 = digest_bytes(HashAlgorithm::Sha256, &bytes);
        assert_eq!(h1, h2);
    }
}
