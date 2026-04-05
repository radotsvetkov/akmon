//! Structured audit events serializable to JSON.

use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::permission::Permission;

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    /// Request permitted under active policy.
    Allow,
    /// Request denied under active policy.
    Deny,
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

/// Writes `events` as [JSON Lines](https://jsonlines.org/): one [`AuditEvent::to_json`] object per line.
///
/// Creates parent directories as needed. Truncates or creates `path` (not append).
pub fn write_audit_jsonl(path: &Path, events: &[AuditEvent]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    for event in events {
        let line = event
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn write_audit_jsonl_produces_valid_jsonl_lines() {
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
        let parsed: AuditEvent = serde_json::from_str(line).expect("deserialize");
        assert_eq!(parsed, ev);
    }
}
