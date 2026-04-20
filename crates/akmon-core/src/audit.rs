//! Structured audit events serializable to JSON.

use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::permission::Permission;

/// Stable JSONL schema marker for chained audit records.
pub const AUDIT_CHAIN_SCHEMA_VERSION: &str = "audit_chain.v1";

/// High-level outcome for tool execution (audit trail).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcomeKind {
    /// Tool finished and reported success.
    Success,
    /// Tool finished with an error or policy block after dispatch.
    Failure,
}

/// Result of a policy check as recorded in the audit log.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    /// Request permitted under active policy.
    Allow,
    /// Request denied under active policy.
    Deny,
}

/// Verdict from an interactive prompt (TUI or stdin), optionally remembering this permission for the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractivePolicyReply {
    /// Whether the user allowed or denied the pending permission.
    pub verdict: PolicyVerdict,
    /// When `true` with [`PolicyVerdict::Allow`], identical [`crate::permission::Permission`] values
    /// may auto-approve for the rest of the agent session without prompting again (orchestrator-defined).
    #[serde(default)]
    pub remember_for_session: bool,
    /// When `true` with allow, any write/edit file permission is auto-approved for this session.
    #[serde(default)]
    pub allow_all_writes_session: bool,
    /// When set with allow for shell, commands with this prefix are auto-approved (session-wide).
    #[serde(default)]
    pub shell_allow_prefix: Option<String>,
}

impl InteractivePolicyReply {
    /// Allow this action once (default for stdin / CLI).
    pub fn allow_once() -> Self {
        Self {
            verdict: PolicyVerdict::Allow,
            remember_for_session: false,
            allow_all_writes_session: false,
            shell_allow_prefix: None,
        }
    }

    /// Deny this action.
    pub fn deny() -> Self {
        Self {
            verdict: PolicyVerdict::Deny,
            remember_for_session: false,
            allow_all_writes_session: false,
            shell_allow_prefix: None,
        }
    }
}

/// One append-only audit record. Serialized with `serde_json` for files and CLI `--output json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event_kind", rename_all = "snake_case")]
pub enum AuditEvent {
    /// Policy engine evaluated a [`Permission`] before work proceeds.
    PolicyEvaluation {
        /// Session identifier (stable for the lifetime of a run).
        session_id: String,
        /// UTC time when evaluation completed.
        timestamp: DateTime<Utc>,
        /// Request that was evaluated.
        permission: Permission,
        /// Outcome of the check.
        verdict: PolicyVerdict,
        /// Human-readable explanation (safe for logs; no secret material).
        reason: String,
        /// MCP server context when this decision is for an MCP action.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_server: Option<String>,
        /// MCP tool context when this decision is for an MCP action.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_tool: Option<String>,
        /// Structured decision reason for downstream machine parsing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_reason: Option<String>,
    },
    /// A tool invocation was dispatched to the execution layer.
    ToolDispatch {
        /// Session identifier.
        session_id: String,
        /// UTC time when dispatch was recorded.
        timestamp: DateTime<Utc>,
        /// Tool name (stable identifier).
        tool_name: String,
        /// Redacted or summarized arguments — never raw API keys.
        input_summary: String,
        /// MCP server context when this dispatch is an MCP tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_server: Option<String>,
        /// MCP tool context when this dispatch is an MCP tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_tool: Option<String>,
        /// Optional decision reason associated with this dispatch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_reason: Option<String>,
    },
    /// A tool invocation completed (success or failure).
    ToolOutcome {
        /// Session identifier.
        session_id: String,
        /// UTC time when the outcome was recorded.
        timestamp: DateTime<Utc>,
        /// Tool name (stable identifier).
        tool_name: String,
        /// Whether the tool reported success.
        outcome: ToolOutcomeKind,
        /// Short, log-safe summary (no secrets).
        summary: String,
        /// MCP server context when this outcome is an MCP tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_server: Option<String>,
        /// MCP tool context when this outcome is an MCP tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_tool: Option<String>,
        /// Optional decision reason associated with this outcome.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_reason: Option<String>,
    },
    /// One observable step from the agent orchestrator (FSM events, stream milestones).
    AgentStep {
        /// Session identifier.
        session_id: String,
        /// UTC time when the step was recorded.
        timestamp: DateTime<Utc>,
        /// Log-safe description (typically the display form of an FSM event).
        description: String,
    },
}

impl AuditEvent {
    /// Serializes this event to a JSON string for audit files or IPC.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Pretty-prints JSON (for debugging and human-readable audit files).
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// One audit JSONL record enriched with tamper-evident chain metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditChainRecord {
    /// Schema marker for downstream parser compatibility.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    /// Zero-based position in the session audit stream.
    pub event_index: u64,
    /// Hash of the previous event (`None` for the first record).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    /// Hash for this record (`sha256(canonical({event, prev_hash}))`).
    pub event_hash: String,
    /// Final session hash, set only on the last record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_final_hash: Option<String>,
    /// Original audit event payload.
    #[serde(flatten)]
    pub event: AuditEvent,
}

fn default_schema_version() -> String {
    String::new()
}

/// Final verification summary for an audit chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditChainSummary {
    /// Number of verified events.
    pub event_count: u64,
    /// Final hash derived from the last event.
    pub session_final_hash: Option<String>,
}

/// Errors produced while building, writing, or verifying audit chains.
#[derive(Debug, Error)]
pub enum AuditChainError {
    /// Input/output failure while reading or writing audit files.
    #[error("audit I/O failure: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization or deserialization failure.
    #[error("audit serialization failure: {0}")]
    Serde(#[from] serde_json::Error),
    /// Chain index mismatch.
    #[error(
        "chain verification failed at line {line}: expected event_index {expected}, got {actual}"
    )]
    EventIndexMismatch {
        /// 1-based line number in JSONL file.
        line: usize,
        /// Expected index for this line.
        expected: u64,
        /// Actual index from the record.
        actual: u64,
    },
    /// Previous hash mismatch.
    #[error("chain verification failed at line {line}: prev_hash mismatch")]
    PrevHashMismatch {
        /// 1-based line number in JSONL file.
        line: usize,
    },
    /// Event hash mismatch.
    #[error("chain verification failed at line {line}: event_hash mismatch")]
    EventHashMismatch {
        /// 1-based line number in JSONL file.
        line: usize,
    },
    /// Terminal session hash mismatch.
    #[error("chain verification failed at line {line}: session_final_hash mismatch")]
    SessionFinalHashMismatch {
        /// 1-based line number in JSONL file.
        line: usize,
    },
    /// Non-terminal record unexpectedly contained `session_final_hash`.
    #[error("chain verification failed at line {line}: non-terminal record has session_final_hash")]
    NonTerminalSessionFinalHash {
        /// 1-based line number in JSONL file.
        line: usize,
    },
    /// Record schema does not match the expected chain schema version.
    #[error(
        "chain verification failed at line {line}: unsupported schema_version `{found}`, expected `{expected}`"
    )]
    UnsupportedSchemaVersion {
        /// 1-based line number in JSONL file.
        line: usize,
        /// Observed schema marker in the record.
        found: String,
        /// Expected schema marker.
        expected: &'static str,
    },
}

/// Builds a deterministic hash chain from in-memory events.
pub fn build_audit_chain(events: &[AuditEvent]) -> Result<Vec<AuditChainRecord>, AuditChainError> {
    let mut records = Vec::with_capacity(events.len());
    let mut prev_hash: Option<String> = None;

    for (idx, event) in events.iter().enumerate() {
        let prev_hash_value = prev_hash.as_deref().unwrap_or_default();
        let event_hash = compute_event_hash(event, prev_hash_value)?;
        records.push(AuditChainRecord {
            schema_version: AUDIT_CHAIN_SCHEMA_VERSION.to_string(),
            event_index: idx as u64,
            prev_hash: prev_hash.clone(),
            event_hash: event_hash.clone(),
            session_final_hash: None,
            event: event.clone(),
        });
        prev_hash = Some(event_hash);
    }

    if let Some(last) = records.last_mut() {
        last.session_final_hash = Some(last.event_hash.clone());
    }

    Ok(records)
}

/// Verifies an already-loaded chain of [`AuditChainRecord`] values.
pub fn verify_audit_chain(
    records: &[AuditChainRecord],
) -> Result<AuditChainSummary, AuditChainError> {
    let mut prev_hash: Option<String> = None;

    for (line_idx, record) in records.iter().enumerate() {
        let line = line_idx + 1;
        let expected_index = line_idx as u64;
        if record.event_index != expected_index {
            return Err(AuditChainError::EventIndexMismatch {
                line,
                expected: expected_index,
                actual: record.event_index,
            });
        }
        if record.schema_version != AUDIT_CHAIN_SCHEMA_VERSION {
            return Err(AuditChainError::UnsupportedSchemaVersion {
                line,
                found: record.schema_version.clone(),
                expected: AUDIT_CHAIN_SCHEMA_VERSION,
            });
        }

        if record.prev_hash != prev_hash {
            return Err(AuditChainError::PrevHashMismatch { line });
        }
        if line_idx + 1 < records.len() && record.session_final_hash.is_some() {
            return Err(AuditChainError::NonTerminalSessionFinalHash { line });
        }

        let expected_hash =
            compute_event_hash(&record.event, prev_hash.as_deref().unwrap_or_default())?;
        if record.event_hash != expected_hash {
            return Err(AuditChainError::EventHashMismatch { line });
        }
        prev_hash = Some(record.event_hash.clone());
    }

    if let Some((line_idx, last)) = records.iter().enumerate().next_back()
        && let Some(summary_hash) = &last.session_final_hash
        && summary_hash != &last.event_hash
    {
        return Err(AuditChainError::SessionFinalHashMismatch { line: line_idx + 1 });
    }

    Ok(AuditChainSummary {
        event_count: records.len() as u64,
        session_final_hash: records.last().map(|r| r.event_hash.clone()),
    })
}

/// Reads and verifies a JSONL audit file.
pub fn verify_audit_jsonl(path: &Path) -> Result<AuditChainSummary, AuditChainError> {
    let raw = std::fs::read_to_string(path)?;
    let mut records = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let record: AuditChainRecord = serde_json::from_str(line)?;
        records.push(record);
    }
    verify_audit_chain(&records)
}

fn compute_event_hash(event: &AuditEvent, prev_hash: &str) -> Result<String, AuditChainError> {
    #[derive(Serialize)]
    struct HashPayload<'a> {
        prev_hash: &'a str,
        event: &'a AuditEvent,
    }

    let payload = HashPayload { prev_hash, event };
    let raw = serde_json::to_value(payload)?;
    let canonical = canonicalize_json(raw);
    let bytes = serde_json::to_vec(&canonical)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let mut ordered = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    ordered.insert(key, canonicalize_json(v.clone()));
                }
            }
            serde_json::Value::Object(ordered)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.into_iter()
                .map(canonicalize_json)
                .collect::<Vec<serde_json::Value>>(),
        ),
        other => other,
    }
}

/// Writes `events` as [JSON Lines](https://jsonlines.org/) with hash-chain metadata.
///
/// Creates parent directories as needed. Truncates or creates `path` (not append).
pub fn write_audit_jsonl(path: &Path, events: &[AuditEvent]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    let records = build_audit_chain(events)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    for record in records {
        let line = serde_json::to_string(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn write_audit_jsonl_produces_valid_chain_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");
        let ev = AuditEvent::AgentStep {
            session_id: "sess-a".into(),
            timestamp: Utc::now(),
            description: "step".into(),
        };
        write_audit_jsonl(&path, std::slice::from_ref(&ev)).expect("write");
        let contents = std::fs::read_to_string(&path).expect("read");
        let line = contents.lines().next().expect("one line");
        let parsed: AuditChainRecord = serde_json::from_str(line).expect("deserialize");
        assert_eq!(parsed.event, ev);
        assert_eq!(parsed.schema_version, AUDIT_CHAIN_SCHEMA_VERSION);
        assert_eq!(parsed.event_index, 0);
        assert!(parsed.prev_hash.is_none());
        assert_eq!(parsed.session_final_hash, Some(parsed.event_hash.clone()));
    }

    #[test]
    fn chain_verification_fails_after_tamper() {
        let now = Utc::now();
        let events = vec![
            AuditEvent::AgentStep {
                session_id: "sess-a".into(),
                timestamp: now,
                description: "first".into(),
            },
            AuditEvent::AgentStep {
                session_id: "sess-a".into(),
                timestamp: now,
                description: "second".into(),
            },
        ];
        let mut chain = build_audit_chain(&events).expect("build chain");
        chain[0].event = AuditEvent::AgentStep {
            session_id: "sess-a".into(),
            timestamp: now,
            description: "tampered".into(),
        };
        let err = verify_audit_chain(&chain).expect_err("tamper should fail");
        assert!(matches!(
            err,
            AuditChainError::EventHashMismatch { line: 1 }
        ));
    }

    #[test]
    fn chain_verification_rejects_non_terminal_summary_hash() {
        let now = Utc::now();
        let events = vec![
            AuditEvent::AgentStep {
                session_id: "sess-a".into(),
                timestamp: now,
                description: "first".into(),
            },
            AuditEvent::AgentStep {
                session_id: "sess-a".into(),
                timestamp: now,
                description: "second".into(),
            },
        ];
        let mut chain = build_audit_chain(&events).expect("build chain");
        chain[0].session_final_hash = Some(chain[0].event_hash.clone());
        let err = verify_audit_chain(&chain).expect_err("must reject non-terminal summary hash");
        assert!(matches!(
            err,
            AuditChainError::NonTerminalSessionFinalHash { line: 1 }
        ));
    }

    #[test]
    fn chain_verification_rejects_unknown_schema_version() {
        let mut chain = build_audit_chain(&[AuditEvent::AgentStep {
            session_id: "sess-a".into(),
            timestamp: Utc::now(),
            description: "first".into(),
        }])
        .expect("build chain");
        chain[0].schema_version = "audit_chain.v999".into();
        let err = verify_audit_chain(&chain).expect_err("must reject unknown schema");
        assert!(matches!(
            err,
            AuditChainError::UnsupportedSchemaVersion { line: 1, .. }
        ));
    }

    #[test]
    fn verify_jsonl_round_trip_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");
        let events = vec![
            AuditEvent::AgentStep {
                session_id: "sess-a".into(),
                timestamp: Utc::now(),
                description: "one".into(),
            },
            AuditEvent::AgentStep {
                session_id: "sess-a".into(),
                timestamp: Utc::now(),
                description: "two".into(),
            },
        ];
        write_audit_jsonl(&path, &events).expect("write");
        let summary = verify_audit_jsonl(&path).expect("verify");
        assert_eq!(summary.event_count, 2);
        assert!(summary.session_final_hash.is_some());
    }
}
