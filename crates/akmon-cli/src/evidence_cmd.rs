//! `akmon evidence` subcommands for evidence artifact verification.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_core::validate_evidence_json;
use clap::Subcommand;
use serde_json::json;

/// Top-level `akmon evidence …` options.
#[derive(Debug, Clone, clap::Args)]
pub struct EvidenceArgs {
    /// Evidence subcommand.
    #[command(subcommand)]
    pub cmd: EvidenceSubcommand,
}

/// Supported `akmon evidence` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum EvidenceSubcommand {
    /// Verify one evidence JSON file and linked audit chain.
    Verify {
        /// Path to evidence JSON file.
        path: PathBuf,
    },
}

/// Runs one `akmon evidence` invocation.
pub fn run_evidence(args: EvidenceArgs, json_output: bool) -> ExitCode {
    match args.cmd {
        EvidenceSubcommand::Verify { path } => match verify_evidence_path(&path) {
            Ok(artifact) => {
                if json_output {
                    let payload = json!({
                        "ok": true,
                        "path": path,
                        "session_id": artifact.session_id,
                        "evidence_schema_version": artifact.evidence_schema_version,
                    });
                    println!("{payload}");
                } else {
                    let displayed = path.display();
                    println!(
                        "evidence verify: valid ({displayed}) session_id={} schema={}",
                        artifact.session_id, artifact.evidence_schema_version
                    );
                }
                ExitCode::SUCCESS
            }
            Err(message) => {
                if json_output {
                    let payload = json!({
                        "ok": false,
                        "path": path,
                        "error": message,
                    });
                    println!("{payload}");
                } else {
                    eprintln!("evidence verify: {message}");
                }
                ExitCode::from(1)
            }
        },
    }
}

/// Verifies one evidence file and returns the parsed artifact.
pub fn verify_evidence_path(path: &Path) -> Result<akmon_core::EvidenceArtifact, String> {
    validate_evidence_json(path)
        .map_err(|e| format!("invalid evidence file {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::{
        AuditEvent, EvidenceArtifact, EvidenceAudit, EvidencePolicy, EvidenceToolCall,
        EvidenceTools, EvidenceVerification, ReplayMetadata, write_audit_jsonl,
        write_evidence_json,
    };
    use chrono::Utc;

    fn sample_replay() -> ReplayMetadata {
        ReplayMetadata {
            hash_algorithm: "sha256".into(),
            provider_name: "ollama".into(),
            model_id: "llama3.2".into(),
            session_id: "sess-e".into(),
            policy_hash: "a".repeat(64),
            config_hash: "b".repeat(64),
            tool_registry_hash: "c".repeat(64),
            prompt_assembly_hash: Some("d".repeat(64)),
        }
    }

    fn write_valid_evidence(path: &Path, audit_path: &Path) {
        write_audit_jsonl(
            audit_path,
            &[AuditEvent::AgentStep {
                session_id: "sess-e".into(),
                timestamp: Utc::now(),
                description: "step".into(),
            }],
        )
        .expect("write audit");
        let summary = akmon_core::verify_audit_jsonl(audit_path).expect("verify audit");
        let artifact = EvidenceArtifact::new(
            "sess-e".into(),
            Utc::now(),
            sample_replay(),
            EvidenceAudit {
                audit_log_path: audit_path.to_string_lossy().into_owned(),
                audit_chain_valid: true,
                session_final_hash: summary.session_final_hash,
            },
            EvidencePolicy {
                allow: 1,
                deny: 0,
                prompted: 0,
                decision_samples: vec![],
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
                outcomes: vec![],
                unavailable_reason: Some("none".into()),
            },
        );
        write_evidence_json(path, &artifact).expect("write evidence");
    }

    #[test]
    fn verify_evidence_path_accepts_valid_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let evidence = dir.path().join("evidence.json");
        let audit = dir.path().join("audit.jsonl");
        write_valid_evidence(&evidence, &audit);
        let parsed = verify_evidence_path(&evidence).expect("valid");
        assert_eq!(parsed.session_id, "sess-e");
    }

    #[test]
    fn verify_evidence_path_rejects_missing_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let evidence = dir.path().join("missing.json");
        let err = verify_evidence_path(&evidence).expect_err("missing");
        assert!(
            err.contains("No such file") || err.contains("os error"),
            "{err}"
        );
    }

    #[test]
    fn run_evidence_returns_failure_for_tampered_replay() {
        let dir = tempfile::tempdir().expect("tmp");
        let evidence = dir.path().join("evidence.json");
        let audit = dir.path().join("audit.jsonl");
        write_valid_evidence(&evidence, &audit);
        let mut parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&evidence).expect("read")).expect("json");
        parsed["replay_metadata"]["policy_hash"] = serde_json::Value::String("broken".into());
        std::fs::write(
            &evidence,
            serde_json::to_string_pretty(&parsed).expect("serialize"),
        )
        .expect("write");
        let code = run_evidence(
            EvidenceArgs {
                cmd: EvidenceSubcommand::Verify {
                    path: evidence.clone(),
                },
            },
            true,
        );
        assert_eq!(code, ExitCode::from(1));
    }
}
