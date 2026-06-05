//! AGEF bundle manifest types and validation.

use crate::BundleError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// AGEF bundle manifest (`manifest.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// AGEF wire version declared by bundle producer.
    pub agef_version: String,
    /// Producer metadata.
    pub producer: Producer,
    /// Session metadata.
    pub session: SessionMetadata,
    /// Hash algorithm name (`sha256` or `blake3` in v0.1.x).
    pub hash_algorithm: String,
    /// Number of objects in `objects/`.
    pub object_count: u64,
    /// Number of events in `events.bin`.
    pub event_count: u64,
    /// Optional detached signatures over the session head (AGEF v0.1.2 §A.14). Absent ⇒ unsigned;
    /// omitted from serialized JSON when `None`, so unsigned manifests are byte-identical to v0.1.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signatures: Option<Vec<ManifestSignature>>,
    /// Optional operator-identity attestations binding a named human/role to the session head
    /// (decision D-20; AGEF v0.1.3 §A.15). Absent ⇒ unattributed; omitted from serialized JSON when
    /// `None`, so manifests without operator attestations are byte-identical to prior versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_attestations: Option<Vec<OperatorAttestation>>,
    /// Forward-compatible extra metadata fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Producer identity fields in `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Producer {
    /// Producer name.
    pub name: String,
    /// Producer version.
    pub version: String,
}

/// Session metadata fields in `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMetadata {
    /// Session UUID (hyphenated lowercase).
    pub id: String,
    /// Head event hash as lowercase hex.
    pub head: String,
    /// RFC3339 start timestamp.
    pub created_at: String,
    /// RFC3339 end timestamp.
    pub ended_at: String,
}

/// A detached signature over the session head (AGEF v0.1.2 §A.14).
///
/// Optional manifest metadata; never part of the event hash chain (decision D-18, S5). A signer
/// covers the canonical `AGEF-SIG-v1` statement (see [`crate::signing::signing_statement`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestSignature {
    /// Signature scheme. `ed25519` is the only scheme defined in AGEF v0.1.2.
    pub scheme: String,
    /// Key identifier: lowercase hex SHA-256 of the signer's raw public key.
    pub key_id: String,
    /// Version tag of the signed statement (`AGEF-SIG-v1`).
    pub statement_version: String,
    /// Detached signature bytes as lowercase hex.
    pub signature: String,
    /// RFC3339 timestamp when the signature was produced.
    pub created_at: String,
}

/// An operator-identity attestation binding a named human and role to the session head
/// (decision D-20; AGEF v0.1.3 §A.15).
///
/// Optional manifest metadata; never part of the event hash chain (decision D-20, additive-only). An
/// attester covers the canonical `AGEF-OPERATOR-v1` statement (see
/// [`crate::signing::operator_statement`]), which binds the four identity fields to the session's
/// `agef_version`, `hash_algorithm`, `session_id`, and `head`. The four identity fields
/// (`operator_id`, `display_name`, `role`, `org`) are part of the signed statement; `created_at` is
/// metadata only and is NOT signed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperatorAttestation {
    /// Signature scheme. `ed25519` is the only scheme defined in AGEF v0.1.3.
    pub scheme: String,
    /// Key identifier: lowercase hex SHA-256 of the attester's raw public key.
    pub key_id: String,
    /// Version tag of the signed statement (`AGEF-OPERATOR-v1`).
    pub statement_version: String,
    /// Stable operator identifier (signed). For example an email, employee id, or service account.
    pub operator_id: String,
    /// Human-readable display name of the operator (signed).
    pub display_name: String,
    /// Role the operator acted in for this session (signed). For example `release-engineer`.
    pub role: String,
    /// Organization the operator belongs to (signed).
    pub org: String,
    /// Detached signature bytes as lowercase hex over the `AGEF-OPERATOR-v1` statement.
    pub signature: String,
    /// RFC3339 timestamp when the attestation was produced. Metadata only; NOT part of the signed
    /// statement.
    pub created_at: String,
}

impl Manifest {
    /// Validates that `session.id` is a hyphenated lowercase UUID.
    pub fn validate_session_id_format(&self) -> Result<(), BundleError> {
        let parsed = uuid::Uuid::parse_str(&self.session.id).map_err(|err| {
            BundleError::InvalidManifest(format!("session.id is not a UUID: {err}"))
        })?;
        let canonical = parsed.hyphenated().to_string();
        if canonical != self.session.id {
            return Err(BundleError::InvalidManifest(
                "session.id must be hyphenated lowercase UUID".to_owned(),
            ));
        }
        Ok(())
    }

    /// Validates that `session.head` is hex and matches the active algorithm digest size.
    pub fn validate_head_format(&self) -> Result<(), BundleError> {
        let expected_len = match self.hash_algorithm.as_str() {
            "sha256" | "blake3" => 64,
            other => {
                return Err(BundleError::UnsupportedHashAlgorithm(other.to_owned()));
            }
        };
        if self.session.head.len() != expected_len {
            return Err(BundleError::InvalidManifest(format!(
                "session.head must be {expected_len} hex characters for {}",
                self.hash_algorithm
            )));
        }
        if !self
            .session
            .head
            .as_bytes()
            .iter()
            .all(u8::is_ascii_hexdigit)
        {
            return Err(BundleError::InvalidManifest(
                "session.head must be lowercase hex".to_owned(),
            ));
        }
        if self.session.head != self.session.head.to_lowercase() {
            return Err(BundleError::InvalidManifest(
                "session.head must be lowercase hex".to_owned(),
            ));
        }
        Ok(())
    }

    /// Validates that `hash_algorithm` is supported by AGEF v0.1.x.
    pub fn validate_hash_algorithm(&self) -> Result<(), BundleError> {
        match self.hash_algorithm.as_str() {
            "sha256" | "blake3" => Ok(()),
            other => Err(BundleError::UnsupportedHashAlgorithm(other.to_owned())),
        }
    }

    /// Validates that `session.created_at` and `session.ended_at` are RFC3339 timestamps.
    pub fn validate_timestamps(&self) -> Result<(), BundleError> {
        time::OffsetDateTime::parse(
            &self.session.created_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|err| {
            BundleError::InvalidManifest(format!("session.created_at is not RFC3339: {err}"))
        })?;
        time::OffsetDateTime::parse(
            &self.session.ended_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|err| {
            BundleError::InvalidManifest(format!("session.ended_at is not RFC3339: {err}"))
        })?;
        Ok(())
    }

    /// Validates aggregate manifest semantics.
    pub fn validate_all(&self) -> Result<(), BundleError> {
        self.validate_session_id_format()?;
        self.validate_hash_algorithm()?;
        self.validate_head_format()?;
        self.validate_timestamps()?;
        Ok(())
    }

    /// Validates AGEF compatibility against expected reader version.
    ///
    /// For v2.0.0, compatibility uses major/minor matching:
    /// - expected `0.1.x` accepts manifest `0.1.y`
    /// - different major/minor (for example `0.2.x`) is rejected
    pub fn validate_agef_version(&self, expected: &str) -> Result<(), BundleError> {
        let (exp_major, exp_minor, _) = parse_semver(expected).ok_or_else(|| {
            BundleError::UnsupportedAgefVersion(format!(
                "invalid expected AGEF version: {expected}"
            ))
        })?;
        let (got_major, got_minor, _) = parse_semver(&self.agef_version)
            .ok_or_else(|| BundleError::UnsupportedAgefVersion(self.agef_version.clone()))?;
        if exp_major == got_major && exp_minor == got_minor {
            return Ok(());
        }
        Err(BundleError::UnsupportedAgefVersion(
            self.agef_version.clone(),
        ))
    }

    /// Serializes manifest as canonical JSON bytes with sorted object keys.
    pub fn to_canonical_json_bytes(&self) -> Result<Vec<u8>, BundleError> {
        let value = serde_json::to_value(self).map_err(|err| {
            BundleError::InvalidManifest(format!("manifest serialize failed: {err}"))
        })?;
        let sorted = sort_json_value(value);
        serde_json::to_vec(&sorted).map_err(|err| {
            BundleError::InvalidManifest(format!("manifest JSON encode failed: {err}"))
        })
    }

    /// Parses manifest from JSON bytes and validates semantic constraints.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, BundleError> {
        let manifest: Manifest = serde_json::from_slice(bytes)
            .map_err(|err| BundleError::InvalidManifest(format!("manifest parse failed: {err}")))?;
        manifest.validate_all()?;
        Ok(manifest)
    }
}

fn parse_semver(input: &str) -> Option<(u64, u64, u64)> {
    let mut parts = input.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let val = map.get(&key).cloned().unwrap_or(Value::Null);
                out.insert(key, sort_json_value(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sort_json_value).collect()),
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            agef_version: akmon_journal::AGEF_SPEC_VERSION.to_owned(),
            producer: Producer {
                name: "akmon".to_owned(),
                version: "2.0.0".to_owned(),
            },
            session: SessionMetadata {
                id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                created_at: "2026-05-04T14:00:00Z".to_owned(),
                ended_at: "2026-05-04T14:01:00Z".to_owned(),
            },
            hash_algorithm: "sha256".to_owned(),
            object_count: 2,
            event_count: 3,
            signatures: None,
            operator_attestations: None,
            extra: BTreeMap::from([("x_extra".to_owned(), serde_json::json!({"z":1,"a":2}))]),
        }
    }

    #[test]
    fn t_manifest_round_trip_canonical_json() {
        let m = manifest();
        let bytes = m.to_canonical_json_bytes().expect("canonical json");
        let parsed = Manifest::from_json_bytes(&bytes).expect("parse");
        assert_eq!(parsed, m);
    }

    #[test]
    fn t_manifest_validate_all_passes_for_well_formed() {
        assert!(manifest().validate_all().is_ok());
    }

    #[test]
    fn t_manifest_validate_session_id_rejects_invalid() {
        let mut m = manifest();
        m.session.id = "not-a-uuid".to_owned();
        assert!(m.validate_session_id_format().is_err());
    }

    #[test]
    fn t_manifest_validate_head_rejects_non_hex() {
        let mut m = manifest();
        m.session.head = "gg".repeat(32);
        assert!(m.validate_head_format().is_err());
    }

    #[test]
    fn t_manifest_validate_hash_algorithm_rejects_unknown() {
        let mut m = manifest();
        m.hash_algorithm = "sha1".to_owned();
        assert!(m.validate_hash_algorithm().is_err());
    }

    #[test]
    fn t_manifest_validate_timestamps_rejects_non_rfc3339() {
        let mut m = manifest();
        m.session.created_at = "2026/05/04 14:00:00".to_owned();
        assert!(m.validate_timestamps().is_err());
    }

    #[test]
    fn t_manifest_validate_agef_version_accepts_compatible() {
        let m = manifest();
        assert!(m.validate_agef_version("0.1.9").is_ok());
    }

    #[test]
    fn t_manifest_validate_agef_version_rejects_incompatible_major() {
        let m = manifest();
        assert!(m.validate_agef_version("0.2.0").is_err());
    }

    #[test]
    fn t_manifest_serde_flatten_preserves_extra_fields() {
        let m = manifest();
        let bytes = serde_json::to_vec(&m).expect("serialize");
        let parsed: Manifest = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(parsed.extra.get("x_extra"), m.extra.get("x_extra"));
    }
}
