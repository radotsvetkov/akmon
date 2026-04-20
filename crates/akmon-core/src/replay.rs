//! Replay metadata primitives for deterministic run reproducibility.
//!
//! Hashes are computed over canonicalized JSON payloads (object keys sorted
//! recursively) to keep digests stable across map insertion order and platforms.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Hash algorithm identifier used by replay metadata.
pub const REPLAY_HASH_ALGORITHM: &str = "sha256";

/// Deterministic replay metadata attached to run outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayMetadata {
    /// Hash algorithm used for all replay hashes.
    pub hash_algorithm: String,
    /// Confirmed provider label used for this run.
    pub provider_name: String,
    /// Provider model id used for completion calls.
    pub model_id: String,
    /// Stable session identifier.
    pub session_id: String,
    /// Hash of the effective non-secret policy representation.
    pub policy_hash: String,
    /// Hash of the effective non-secret runtime config representation.
    pub config_hash: String,
    /// Hash of the active tool registry snapshot.
    pub tool_registry_hash: String,
    /// Optional hash of prompt-assembly shape (structure only, no raw text).
    pub prompt_assembly_hash: Option<String>,
}

/// Canonical replay hash inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayHashInputs {
    /// Canonical value describing effective policy state.
    pub policy: Value,
    /// Canonical value describing effective non-secret config state.
    pub config: Value,
    /// Canonical value describing registered tools.
    pub tool_registry: Value,
    /// Optional prompt assembly fingerprint (no raw prompt text).
    pub prompt_assembly: Option<Value>,
}

/// Replay metadata validation failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplayMetadataError {
    /// Unsupported hash algorithm.
    #[error("unsupported hash_algorithm `{found}`, expected `{expected}`")]
    UnsupportedHashAlgorithm {
        /// Observed algorithm value.
        found: String,
        /// Expected algorithm value.
        expected: &'static str,
    },
    /// Required field is missing/empty.
    #[error("missing required replay metadata field `{field}`")]
    MissingField {
        /// Name of missing field.
        field: &'static str,
    },
    /// One hash field has invalid format.
    #[error("invalid hash format for `{field}`")]
    InvalidHashFormat {
        /// Field name.
        field: &'static str,
    },
    /// Hash mismatch for recomputed replay inputs.
    #[error("replay hash mismatch for `{field}`")]
    HashMismatch {
        /// Field name.
        field: &'static str,
    },
    /// Serialization failure while hashing.
    #[error("failed to serialize replay hash input: {0}")]
    Serialization(String),
}

/// Computes a deterministic SHA-256 hex digest from canonical JSON for `value`.
pub fn canonical_json_sha256(value: &Value) -> Result<String, ReplayMetadataError> {
    let canonical = canonicalize_json(value.clone());
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|e| ReplayMetadataError::Serialization(e.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Builds replay metadata from stable identity fields and canonical hash inputs.
pub fn build_replay_metadata(
    provider_name: &str,
    model_id: &str,
    session_id: &str,
    inputs: &ReplayHashInputs,
) -> Result<ReplayMetadata, ReplayMetadataError> {
    let policy_hash = canonical_json_sha256(&inputs.policy)?;
    let config_hash = canonical_json_sha256(&inputs.config)?;
    let tool_registry_hash = canonical_json_sha256(&inputs.tool_registry)?;
    let prompt_assembly_hash = match &inputs.prompt_assembly {
        Some(v) => Some(canonical_json_sha256(v)?),
        None => None,
    };
    Ok(ReplayMetadata {
        hash_algorithm: REPLAY_HASH_ALGORITHM.to_string(),
        provider_name: provider_name.to_string(),
        model_id: model_id.to_string(),
        session_id: session_id.to_string(),
        policy_hash,
        config_hash,
        tool_registry_hash,
        prompt_assembly_hash,
    })
}

/// Validates required fields and hash formatting for one metadata object.
pub fn validate_replay_metadata(metadata: &ReplayMetadata) -> Result<(), ReplayMetadataError> {
    if metadata.hash_algorithm != REPLAY_HASH_ALGORITHM {
        return Err(ReplayMetadataError::UnsupportedHashAlgorithm {
            found: metadata.hash_algorithm.clone(),
            expected: REPLAY_HASH_ALGORITHM,
        });
    }
    if metadata.provider_name.trim().is_empty() {
        return Err(ReplayMetadataError::MissingField {
            field: "provider_name",
        });
    }
    if metadata.model_id.trim().is_empty() {
        return Err(ReplayMetadataError::MissingField { field: "model_id" });
    }
    if metadata.session_id.trim().is_empty() {
        return Err(ReplayMetadataError::MissingField {
            field: "session_id",
        });
    }
    validate_sha256_hex("policy_hash", &metadata.policy_hash)?;
    validate_sha256_hex("config_hash", &metadata.config_hash)?;
    validate_sha256_hex("tool_registry_hash", &metadata.tool_registry_hash)?;
    if let Some(v) = &metadata.prompt_assembly_hash {
        validate_sha256_hex("prompt_assembly_hash", v)?;
    }
    Ok(())
}

/// Recomputes hashes for `inputs` and checks they match `metadata`.
pub fn validate_replay_metadata_integrity(
    metadata: &ReplayMetadata,
    inputs: &ReplayHashInputs,
) -> Result<(), ReplayMetadataError> {
    validate_replay_metadata(metadata)?;
    let expected = build_replay_metadata(
        metadata.provider_name.as_str(),
        metadata.model_id.as_str(),
        metadata.session_id.as_str(),
        inputs,
    )?;
    if metadata.policy_hash != expected.policy_hash {
        return Err(ReplayMetadataError::HashMismatch {
            field: "policy_hash",
        });
    }
    if metadata.config_hash != expected.config_hash {
        return Err(ReplayMetadataError::HashMismatch {
            field: "config_hash",
        });
    }
    if metadata.tool_registry_hash != expected.tool_registry_hash {
        return Err(ReplayMetadataError::HashMismatch {
            field: "tool_registry_hash",
        });
    }
    if metadata.prompt_assembly_hash != expected.prompt_assembly_hash {
        return Err(ReplayMetadataError::HashMismatch {
            field: "prompt_assembly_hash",
        });
    }
    Ok(())
}

fn validate_sha256_hex(field: &'static str, value: &str) -> Result<(), ReplayMetadataError> {
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ReplayMetadataError::InvalidHashFormat { field });
    }
    Ok(())
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut ordered = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    ordered.insert(key, canonicalize_json(v.clone()));
                }
            }
            Value::Object(ordered)
        }
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(canonicalize_json).collect::<Vec<_>>())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_inputs() -> ReplayHashInputs {
        ReplayHashInputs {
            policy: json!({"mode":"configured","rules":{"allow":["src/**"]}}),
            config: json!({"max_iterations":25,"plan_mode":false}),
            tool_registry: json!([
                {"name":"read_file","permissions":["read_file"]},
                {"name":"write_file","permissions":["write_file"]}
            ]),
            prompt_assembly: Some(json!({
                "roles":["system","user"],
                "tool_names":["read_file","write_file"],
                "message_count":2
            })),
        }
    }

    #[test]
    fn deterministic_hash_same_input_same_output() {
        let inputs = sample_inputs();
        let m1 = build_replay_metadata("ollama", "llama3.2", "sess-1", &inputs).expect("build");
        let m2 = build_replay_metadata("ollama", "llama3.2", "sess-1", &inputs).expect("build");
        assert_eq!(m1, m2);
    }

    #[test]
    fn sensitivity_policy_change_changes_policy_hash() {
        let inputs = sample_inputs();
        let baseline =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &inputs).expect("build");
        let mut changed = sample_inputs();
        changed.policy =
            json!({"mode":"configured","rules":{"allow":["src/**"],"deny":["src/secrets/**"]}});
        let changed_meta =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &changed).expect("build");
        assert_ne!(baseline.policy_hash, changed_meta.policy_hash);
        assert_eq!(baseline.config_hash, changed_meta.config_hash);
        assert_eq!(baseline.tool_registry_hash, changed_meta.tool_registry_hash);
    }

    #[test]
    fn sensitivity_config_and_tool_registry_changes_change_corresponding_hashes() {
        let inputs = sample_inputs();
        let baseline =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &inputs).expect("build");
        let mut changed_config = sample_inputs();
        changed_config.config = json!({"max_iterations":26,"plan_mode":false});
        let changed_config_meta =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &changed_config).expect("build");
        assert_ne!(baseline.config_hash, changed_config_meta.config_hash);
        assert_eq!(
            baseline.tool_registry_hash,
            changed_config_meta.tool_registry_hash
        );

        let mut changed_tools = sample_inputs();
        changed_tools.tool_registry = json!([
            {"name":"read_file","permissions":["read_file"]},
            {"name":"search","permissions":["list_directory"]}
        ]);
        let changed_tools_meta =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &changed_tools).expect("build");
        assert_ne!(
            baseline.tool_registry_hash,
            changed_tools_meta.tool_registry_hash
        );
    }

    #[test]
    fn canonicalization_ignores_map_key_order() {
        let v1 = json!({"b":2,"a":{"z":1,"y":2}});
        let v2 = json!({"a":{"y":2,"z":1},"b":2});
        let h1 = canonical_json_sha256(&v1).expect("hash");
        let h2 = canonical_json_sha256(&v2).expect("hash");
        assert_eq!(h1, h2);
    }

    #[test]
    fn secret_safety_metadata_contains_no_raw_secret_values() {
        let secret = "sk-live-secret-value";
        let inputs = ReplayHashInputs {
            config: json!({"api_key":"<redacted>","region":"us-east-1"}),
            ..sample_inputs()
        };
        let metadata =
            build_replay_metadata("openai", "gpt-4o-mini", "sess-secret", &inputs).expect("build");
        let serialized = serde_json::to_string(&metadata).expect("serialize");
        assert!(
            !serialized.contains(secret),
            "metadata should never expose raw secrets"
        );
    }

    #[test]
    fn replay_metadata_roundtrip_serde() {
        let metadata =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &sample_inputs()).expect("build");
        let encoded = serde_json::to_string(&metadata).expect("serialize");
        let decoded: ReplayMetadata = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, metadata);
    }

    #[test]
    fn replay_hash_inputs_roundtrip_serde() {
        let inputs = sample_inputs();
        let encoded = serde_json::to_string(&inputs).expect("serialize");
        let decoded: ReplayHashInputs = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, inputs);
    }

    #[test]
    fn validate_integrity_detects_mismatch() {
        let inputs = sample_inputs();
        let mut metadata =
            build_replay_metadata("ollama", "llama3.2", "sess-1", &inputs).expect("build");
        metadata.policy_hash = "0".repeat(64);
        let err = validate_replay_metadata_integrity(&metadata, &inputs).expect_err("mismatch");
        assert_eq!(
            err,
            ReplayMetadataError::HashMismatch {
                field: "policy_hash"
            }
        );
    }
}
