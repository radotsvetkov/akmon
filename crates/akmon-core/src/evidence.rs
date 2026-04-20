//! Deterministic run evidence artifact schema and validation helpers.
//!
//! Evidence artifacts are intended for CI/PR pipelines and correlate replay
//! metadata, audit-chain integrity, policy outcomes, tools, and touched files.

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ReplayMetadata, ReplayMetadataError, RunReliabilityMetrics, validate_replay_metadata,
    verify_audit_jsonl,
};

/// Stable schema version for evidence artifacts.
pub const EVIDENCE_SCHEMA_VERSION: &str = "evidence.v1";

/// Top-level deterministic evidence artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceArtifact {
    /// Evidence schema version marker.
    pub evidence_schema_version: String,
    /// Session identifier for this run.
    pub session_id: String,
    /// UTC generation timestamp.
    pub generated_at: DateTime<Utc>,
    /// Deterministic replay metadata for this run.
    pub replay_metadata: ReplayMetadata,
    /// Audit-chain linkage and validation summary.
    pub audit: EvidenceAudit,
    /// Policy summary for the run.
    pub policy: EvidencePolicy,
    /// Tool timeline and aggregates.
    pub tools: EvidenceTools,
    /// Deterministic sorted unique sandbox-relative file paths touched in run.
    pub files_touched: Vec<String>,
    /// Verification command outcomes collected by the runtime, if any.
    pub verification: EvidenceVerification,
    /// Reliability/SLO counters for this run.
    #[serde(default)]
    pub reliability_metrics: RunReliabilityMetrics,
    /// Non-fatal collection gaps or warnings.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Audit correlation section for evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceAudit {
    /// Path to session audit JSONL.
    pub audit_log_path: String,
    /// Whether audit-chain validation ran and passed.
    pub audit_chain_valid: bool,
    /// Final hash from audit chain, when available.
    pub session_final_hash: Option<String>,
}

/// Policy summary for evidence output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidencePolicy {
    /// Number of allow decisions.
    pub allow: u64,
    /// Number of deny decisions.
    pub deny: u64,
    /// Number of interactive/prompted decisions (best-effort).
    pub prompted: u64,
    /// Optional sampled decision summaries (sanitized).
    #[serde(default)]
    pub decision_samples: Vec<String>,
}

/// One tool call timeline row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceToolCall {
    /// Tool name.
    pub name: String,
    /// Whether tool execution succeeded.
    pub success: bool,
    /// Tool outcome message.
    pub message: String,
}

/// Tool section with timeline and aggregate counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceTools {
    /// Chronological tool timeline (semantic order preserved).
    pub timeline: Vec<EvidenceToolCall>,
    /// Total number of tool calls.
    pub total: u64,
    /// Number of successful tool calls.
    pub success: u64,
    /// Number of failed tool calls.
    pub failure: u64,
}

/// Verification command outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceVerificationOutcome {
    /// Friendly command identifier.
    pub name: String,
    /// Status string (`success`, `failure`, `skipped`).
    pub status: String,
    /// Process exit code, when available.
    pub exit_code: Option<i32>,
    /// Short command summary.
    pub summary: String,
}

/// Verification section for evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceVerification {
    /// Collected verification outcomes.
    pub outcomes: Vec<EvidenceVerificationOutcome>,
    /// Explicit reason when outcomes are unavailable.
    pub unavailable_reason: Option<String>,
}

/// Validation failures for evidence artifacts.
#[derive(Debug, Error)]
pub enum EvidenceValidationError {
    /// I/O failure while loading or writing evidence.
    #[error("evidence I/O failure: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization/deserialization failure.
    #[error("evidence serialization failure: {0}")]
    Serde(#[from] serde_json::Error),
    /// Unsupported schema marker.
    #[error("unsupported evidence_schema_version `{found}`, expected `{expected}`")]
    UnsupportedSchemaVersion {
        /// Observed schema version.
        found: String,
        /// Expected schema version.
        expected: &'static str,
    },
    /// Missing required field content.
    #[error("missing required evidence field `{field}`")]
    MissingField {
        /// Missing field name.
        field: &'static str,
    },
    /// Replay metadata validation failed.
    #[error("invalid replay metadata: {0}")]
    Replay(#[from] ReplayMetadataError),
    /// Audit validation failed.
    #[error("invalid referenced audit chain: {0}")]
    Audit(String),
    /// Evidence and audit session hashes disagree.
    #[error("session_final_hash mismatch between evidence and audit chain")]
    SessionFinalHashMismatch,
}

impl EvidenceArtifact {
    /// Builds an empty skeleton artifact with schema marker.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: String,
        generated_at: DateTime<Utc>,
        replay_metadata: ReplayMetadata,
        audit: EvidenceAudit,
        policy: EvidencePolicy,
        tools: EvidenceTools,
        files_touched: Vec<String>,
        verification: EvidenceVerification,
    ) -> Self {
        let mut out = Self {
            evidence_schema_version: EVIDENCE_SCHEMA_VERSION.to_string(),
            session_id,
            generated_at,
            replay_metadata,
            audit,
            policy,
            tools,
            files_touched,
            verification,
            reliability_metrics: RunReliabilityMetrics::default(),
            notes: Vec::new(),
        };
        out.normalize_deterministic();
        out
    }

    /// Applies deterministic normalization for stable CI diffs.
    pub fn normalize_deterministic(&mut self) {
        self.files_touched.sort();
        self.files_touched.dedup();
        self.policy.decision_samples.sort();
        self.policy.decision_samples.dedup();
    }
}

/// Writes one evidence artifact JSON file.
pub fn write_evidence_json(path: &Path, artifact: &EvidenceArtifact) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    let json = serde_json::to_string_pretty(artifact)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(file, "{json}")?;
    Ok(())
}

/// Loads and validates an evidence artifact from `path`.
pub fn validate_evidence_json(path: &Path) -> Result<EvidenceArtifact, EvidenceValidationError> {
    let raw = std::fs::read_to_string(path)?;
    let artifact: EvidenceArtifact = serde_json::from_str(&raw)?;
    validate_evidence_artifact(&artifact)?;
    Ok(artifact)
}

/// Validates evidence field integrity and referenced audit chain.
pub fn validate_evidence_artifact(
    artifact: &EvidenceArtifact,
) -> Result<(), EvidenceValidationError> {
    if artifact.evidence_schema_version != EVIDENCE_SCHEMA_VERSION {
        return Err(EvidenceValidationError::UnsupportedSchemaVersion {
            found: artifact.evidence_schema_version.clone(),
            expected: EVIDENCE_SCHEMA_VERSION,
        });
    }
    if artifact.session_id.trim().is_empty() {
        return Err(EvidenceValidationError::MissingField {
            field: "session_id",
        });
    }
    if artifact.audit.audit_log_path.trim().is_empty() {
        return Err(EvidenceValidationError::MissingField {
            field: "audit.audit_log_path",
        });
    }
    validate_replay_metadata(&artifact.replay_metadata)?;
    if artifact.replay_metadata.session_id != artifact.session_id {
        return Err(EvidenceValidationError::MissingField {
            field: "replay_metadata.session_id (must match session_id)",
        });
    }
    let audit_path = PathBuf::from(&artifact.audit.audit_log_path);
    let summary = verify_audit_jsonl(&audit_path).map_err(|e| {
        EvidenceValidationError::Audit(format!("{} ({})", e, audit_path.to_string_lossy()))
    })?;
    if !artifact.audit.audit_chain_valid {
        return Err(EvidenceValidationError::Audit(
            "audit_chain_valid is false".to_string(),
        ));
    }
    if artifact.audit.audit_chain_valid
        && artifact.audit.session_final_hash != summary.session_final_hash
    {
        return Err(EvidenceValidationError::SessionFinalHashMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditEvent, build_audit_chain, write_audit_jsonl};
    use chrono::Utc;

    fn sample_replay() -> ReplayMetadata {
        ReplayMetadata {
            hash_algorithm: "sha256".into(),
            provider_name: "ollama".into(),
            model_id: "llama3.2".into(),
            session_id: "sess-1".into(),
            policy_hash: "a".repeat(64),
            config_hash: "b".repeat(64),
            tool_registry_hash: "c".repeat(64),
            prompt_assembly_hash: Some("d".repeat(64)),
        }
    }

    #[test]
    fn evidence_serialization_shape_contains_required_sections() {
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: "/tmp/audit.jsonl".into(),
                audit_chain_valid: true,
                session_final_hash: Some("f".repeat(64)),
            },
            EvidencePolicy {
                allow: 1,
                deny: 2,
                prompted: 1,
                decision_samples: vec!["allow read_file".into()],
            },
            EvidenceTools {
                timeline: vec![EvidenceToolCall {
                    name: "read_file".into(),
                    success: true,
                    message: "ok".into(),
                }],
                total: 1,
                success: 1,
                failure: 0,
            },
            vec!["src/main.rs".into()],
            EvidenceVerification {
                outcomes: Vec::new(),
                unavailable_reason: Some("not collected".into()),
            },
        );
        let v = serde_json::to_value(&artifact).expect("serialize");
        assert_eq!(v["evidence_schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert!(v.get("replay_metadata").is_some());
        assert!(v.get("audit").is_some());
        assert!(v.get("tools").is_some());
        assert!(v.get("verification").is_some());
        assert!(v.get("reliability_metrics").is_some());
    }

    #[test]
    fn evidence_normalization_sorts_and_dedups_files() {
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: "/tmp/audit.jsonl".into(),
                audit_chain_valid: false,
                session_final_hash: None,
            },
            EvidencePolicy {
                allow: 0,
                deny: 0,
                prompted: 0,
                decision_samples: vec!["b".into(), "a".into(), "a".into()],
            },
            EvidenceTools {
                timeline: Vec::new(),
                total: 0,
                success: 0,
                failure: 0,
            },
            vec!["z.rs".into(), "a.rs".into(), "a.rs".into()],
            EvidenceVerification {
                outcomes: Vec::new(),
                unavailable_reason: Some("none".into()),
            },
        );
        assert_eq!(
            artifact.files_touched,
            vec!["a.rs".to_string(), "z.rs".to_string()]
        );
        assert_eq!(
            artifact.policy.decision_samples,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn evidence_secret_fixture_not_leaked() {
        let secret = "sk-evidence-secret";
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: "/tmp/audit.jsonl".into(),
                audit_chain_valid: false,
                session_final_hash: None,
            },
            EvidencePolicy {
                allow: 0,
                deny: 0,
                prompted: 0,
                decision_samples: vec!["allow read_file".into()],
            },
            EvidenceTools {
                timeline: vec![EvidenceToolCall {
                    name: "read_file".into(),
                    success: true,
                    message: "ok".into(),
                }],
                total: 1,
                success: 1,
                failure: 0,
            },
            vec!["src/main.rs".into()],
            EvidenceVerification {
                outcomes: Vec::new(),
                unavailable_reason: Some("none".into()),
            },
        );
        let encoded = serde_json::to_string(&artifact).expect("serialize");
        assert!(!encoded.contains(secret));
    }

    #[test]
    fn write_and_validate_evidence_json_roundtrip() {
        let dir = tempfile::tempdir().expect("tmp");
        let audit_path = dir.path().join("audit.jsonl");
        let events = vec![AuditEvent::AgentStep {
            session_id: "sess-1".into(),
            timestamp: Utc::now(),
            description: "step".into(),
        }];
        write_audit_jsonl(&audit_path, &events).expect("write audit");
        let audit_summary = crate::verify_audit_jsonl(&audit_path).expect("verify audit");

        let evidence_path = dir.path().join("evidence.json");
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: audit_path.to_string_lossy().into_owned(),
                audit_chain_valid: true,
                session_final_hash: audit_summary.session_final_hash,
            },
            EvidencePolicy {
                allow: 1,
                deny: 0,
                prompted: 0,
                decision_samples: vec![],
            },
            EvidenceTools {
                timeline: Vec::new(),
                total: 0,
                success: 0,
                failure: 0,
            },
            vec![],
            EvidenceVerification {
                outcomes: Vec::new(),
                unavailable_reason: Some("none".into()),
            },
        );
        write_evidence_json(&evidence_path, &artifact).expect("write evidence");
        let parsed = validate_evidence_json(&evidence_path).expect("validate evidence");
        assert_eq!(parsed.session_id, "sess-1");
    }

    #[test]
    fn evidence_links_replay_and_audit_final_hash() {
        let dir = tempfile::tempdir().expect("tmp");
        let audit_path = dir.path().join("audit.jsonl");
        write_audit_jsonl(
            &audit_path,
            &[AuditEvent::AgentStep {
                session_id: "sess-1".into(),
                timestamp: Utc::now(),
                description: "step".into(),
            }],
        )
        .expect("write");
        let summary = crate::verify_audit_jsonl(&audit_path).expect("verify");
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: audit_path.to_string_lossy().into_owned(),
                audit_chain_valid: true,
                session_final_hash: summary.session_final_hash.clone(),
            },
            EvidencePolicy {
                allow: 1,
                deny: 0,
                prompted: 0,
                decision_samples: vec![],
            },
            EvidenceTools {
                timeline: vec![],
                total: 0,
                success: 0,
                failure: 0,
            },
            vec![],
            EvidenceVerification {
                outcomes: vec![],
                unavailable_reason: Some("none".into()),
            },
        );
        assert_eq!(artifact.replay_metadata.hash_algorithm, "sha256");
        assert_eq!(
            artifact.audit.session_final_hash,
            summary.session_final_hash
        );
    }

    #[test]
    fn validate_evidence_fails_for_tampered_replay() {
        let dir = tempfile::tempdir().expect("tmp");
        let audit_path = dir.path().join("audit.jsonl");
        write_audit_jsonl(
            &audit_path,
            &[AuditEvent::AgentStep {
                session_id: "sess-1".into(),
                timestamp: Utc::now(),
                description: "step".into(),
            }],
        )
        .expect("write");
        let mut replay = sample_replay();
        replay.policy_hash = "not-a-hash".into();
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            replay,
            EvidenceAudit {
                audit_log_path: audit_path.to_string_lossy().into_owned(),
                audit_chain_valid: true,
                session_final_hash: crate::verify_audit_jsonl(&audit_path)
                    .expect("verify")
                    .session_final_hash,
            },
            EvidencePolicy {
                allow: 0,
                deny: 0,
                prompted: 0,
                decision_samples: vec![],
            },
            EvidenceTools {
                timeline: Vec::new(),
                total: 0,
                success: 0,
                failure: 0,
            },
            vec![],
            EvidenceVerification {
                outcomes: Vec::new(),
                unavailable_reason: Some("none".into()),
            },
        );
        let err = validate_evidence_artifact(&artifact).expect_err("should fail");
        assert!(matches!(err, EvidenceValidationError::Replay(_)));
    }

    #[test]
    fn validate_evidence_fails_for_invalid_audit_chain() {
        let dir = tempfile::tempdir().expect("tmp");
        let audit_path = dir.path().join("audit.jsonl");
        let mut chain = build_audit_chain(&[AuditEvent::AgentStep {
            session_id: "sess-1".into(),
            timestamp: Utc::now(),
            description: "step".into(),
        }])
        .expect("build");
        chain[0].event = AuditEvent::AgentStep {
            session_id: "sess-1".into(),
            timestamp: Utc::now(),
            description: "tampered".into(),
        };
        let mut raw = String::new();
        for r in chain {
            let line = serde_json::to_string(&r).expect("serialize");
            raw.push_str(&line);
            raw.push('\n');
        }
        std::fs::write(&audit_path, raw).expect("write");
        let artifact = EvidenceArtifact::new(
            "sess-1".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: audit_path.to_string_lossy().into_owned(),
                audit_chain_valid: true,
                session_final_hash: Some("0".repeat(64)),
            },
            EvidencePolicy {
                allow: 0,
                deny: 0,
                prompted: 0,
                decision_samples: vec![],
            },
            EvidenceTools {
                timeline: Vec::new(),
                total: 0,
                success: 0,
                failure: 0,
            },
            vec![],
            EvidenceVerification {
                outcomes: Vec::new(),
                unavailable_reason: Some("none".into()),
            },
        );
        let err = validate_evidence_artifact(&artifact).expect_err("invalid audit");
        assert!(matches!(err, EvidenceValidationError::Audit(_)));
    }
}
