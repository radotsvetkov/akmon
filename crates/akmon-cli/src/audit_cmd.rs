//! `akmon audit` subcommands for audit-chain verification.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_core::verify_audit_jsonl;
use clap::Subcommand;
use serde_json::json;

/// Top-level `akmon audit …` options.
#[derive(Debug, Clone, clap::Args)]
pub struct AuditArgs {
    /// Audit subcommand.
    #[command(subcommand)]
    pub cmd: AuditSubcommand,
}

/// Supported `akmon audit` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum AuditSubcommand {
    /// Verify one audit JSONL file for hash-chain integrity.
    Verify {
        /// Path to audit JSONL file.
        path: PathBuf,
    },
}

/// Runs one `akmon audit` invocation.
pub fn run_audit(args: AuditArgs, json_output: bool) -> ExitCode {
    match args.cmd {
        AuditSubcommand::Verify { path } => match verify_audit_path(&path) {
            Ok(summary) => {
                if json_output {
                    let payload = json!({
                        "ok": true,
                        "path": path,
                        "event_count": summary.event_count,
                        "session_final_hash": summary.session_final_hash,
                    });
                    println!("{payload}");
                } else {
                    let hash = summary
                        .session_final_hash
                        .as_deref()
                        .unwrap_or("<none-for-empty-audit>");
                    let displayed = path.display();
                    println!(
                        "audit verify: valid ({displayed}) events={} final_hash={hash}",
                        summary.event_count
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
                    eprintln!("audit verify: {message}");
                }
                ExitCode::from(1)
            }
        },
    }
}

/// Verifies one audit file and returns the parsed chain summary.
pub fn verify_audit_path(path: &Path) -> Result<akmon_core::AuditChainSummary, String> {
    verify_audit_jsonl(path).map_err(|e| format!("invalid audit file {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::{AuditEvent, build_audit_chain, write_audit_jsonl};
    use chrono::Utc;

    fn sample_events() -> Vec<AuditEvent> {
        vec![
            AuditEvent::AgentStep {
                session_id: "sess-cli".into(),
                timestamp: Utc::now(),
                description: "first".into(),
            },
            AuditEvent::AgentStep {
                session_id: "sess-cli".into(),
                timestamp: Utc::now(),
                description: "second".into(),
            },
        ]
    }

    #[test]
    fn verify_audit_path_accepts_valid_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("audit.jsonl");
        write_audit_jsonl(&path, &sample_events()).expect("write");
        let summary = verify_audit_path(&path).expect("valid");
        assert_eq!(summary.event_count, 2);
        assert!(summary.session_final_hash.is_some());
        let code = run_audit(
            AuditArgs {
                cmd: AuditSubcommand::Verify { path: path.clone() },
            },
            true,
        );
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn verify_audit_path_rejects_tampered_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("audit.jsonl");
        let mut chain = build_audit_chain(&sample_events()).expect("build");
        chain[0].event = AuditEvent::AgentStep {
            session_id: "sess-cli".into(),
            timestamp: Utc::now(),
            description: "tampered".into(),
        };
        let mut raw = String::new();
        for record in chain {
            let line = serde_json::to_string(&record).expect("serialize");
            raw.push_str(&line);
            raw.push('\n');
        }
        std::fs::write(&path, raw).expect("write tampered");

        let err = verify_audit_path(&path).expect_err("tamper must fail");
        assert!(err.contains("event_hash mismatch"), "{err}");
        let code = run_audit(
            AuditArgs {
                cmd: AuditSubcommand::Verify { path: path.clone() },
            },
            true,
        );
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn verify_audit_path_rejects_unknown_schema() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("audit.jsonl");
        let mut chain = build_audit_chain(&sample_events()).expect("build");
        chain[0].schema_version = "audit_chain.v999".into();
        let mut raw = String::new();
        for record in chain {
            let line = serde_json::to_string(&record).expect("serialize");
            raw.push_str(&line);
            raw.push('\n');
        }
        std::fs::write(&path, raw).expect("write unknown schema");

        let err = verify_audit_path(&path).expect_err("unknown schema must fail");
        assert!(err.contains("unsupported schema_version"), "{err}");
        let code = run_audit(
            AuditArgs {
                cmd: AuditSubcommand::Verify { path: path.clone() },
            },
            true,
        );
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn verify_audit_path_rejects_missing_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("missing.jsonl");
        let err = verify_audit_path(&path).expect_err("missing must fail");
        assert!(
            err.contains("No such file") || err.contains("os error"),
            "{err}"
        );
        let code = run_audit(
            AuditArgs {
                cmd: AuditSubcommand::Verify { path },
            },
            true,
        );
        assert_eq!(code, ExitCode::from(1));
    }
}
