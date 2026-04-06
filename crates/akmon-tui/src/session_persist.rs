//! Persist TUI transcripts under `~/.akmon/sessions/` for later resume (slice 4).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::TuiLaunchConfig;
use crate::message::TuiMessage;
use crate::TuiApp;

#[derive(Debug, Serialize)]
struct PersistedSession {
    session_id: String,
    project_root: String,
    model: String,
    started_at: String,
    messages: Vec<PersistedMessage>,
    total_input_tokens: u32,
    total_cache_read_tokens: u32,
    total_output_tokens: u32,
}

#[derive(Debug, Serialize)]
struct PersistedMessage {
    role: String,
    content: String,
}

/// Summary row for `/sessions` and session-picker overlays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Full session UUID string from the saved JSON.
    pub session_id: String,
    /// RFC3339 `started_at` from file.
    pub started_at: String,
    /// First user message preview (up to 60 characters).
    pub first_message: String,
    /// Number of transcript rows stored (user + assistant).
    pub message_count: usize,
}

#[derive(Debug, Deserialize)]
struct SessionFile {
    session_id: String,
    project_root: String,
    model: String,
    started_at: String,
    messages: Vec<SessionFileMsg>,
}

#[derive(Debug, Deserialize)]
struct SessionFileMsg {
    role: String,
    content: String,
}

/// Default JSONL audit path under `project_root`: `.akmon/audit/{session_id}.jsonl`.
pub fn default_audit_log_path(project_root: &Path, session_id: Uuid) -> PathBuf {
    project_root
        .join(".akmon")
        .join("audit")
        .join(format!("{session_id}.jsonl"))
}

/// Reads `*.json` session snapshots from `dir`, newest-first by `started_at` (best-effort).
///
/// Unreadable or invalid files are skipped without error.
pub fn load_session_summaries(dir: &Path) -> Vec<SessionSummary> {
    let mut out: Vec<SessionSummary> = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        let is_json = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        if !is_json {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = serde_json::from_str::<SessionFile>(&raw) else {
            continue;
        };
        let first_message = doc
            .messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.chars().take(60).collect::<String>())
            .unwrap_or_default();
        out.push(SessionSummary {
            session_id: doc.session_id,
            started_at: doc.started_at,
            first_message,
            message_count: doc.messages.len(),
        });
    }
    out.sort_by(|a, b| {
        let ta = DateTime::parse_from_rfc3339(&a.started_at).ok();
        let tb = DateTime::parse_from_rfc3339(&b.started_at).ok();
        match (ta, tb) {
            (Some(aa), Some(bb)) => bb.cmp(&aa),
            _ => b.started_at.cmp(&a.started_at),
        }
    });
    out
}

/// Resolves a session id string, allowing a unique UUID prefix match against known summaries.
pub fn resolve_session_id(input: &str, summaries: &[SessionSummary]) -> Option<String> {
    let needle = input.trim();
    if needle.is_empty() {
        return None;
    }
    if let Ok(u) = Uuid::parse_str(needle) {
        return Some(u.to_string());
    }
    let matches: Vec<&SessionSummary> = summaries
        .iter()
        .filter(|s| s.session_id.starts_with(needle))
        .collect();
    if matches.len() == 1 {
        return Some(matches[0].session_id.clone());
    }
    None
}

/// Loaded session snapshot for `/resume` (transcript + metadata from JSON).
#[derive(Debug, Clone)]
pub struct LoadedSession {
    /// Session id from the snapshot file.
    pub session_id: Uuid,
    /// Project root path stored in the snapshot.
    pub project_root: PathBuf,
    /// Model identifier from the snapshot.
    pub model_name: String,
    /// Parsed `started_at` timestamp from the snapshot (falls back to “now” if invalid).
    pub started_at: DateTime<Utc>,
    /// Transcript rows deserialized into UI messages.
    pub messages: Vec<TuiMessage>,
}

/// Loads a session JSON file into UI messages and metadata.
pub fn load_session_file(path: &Path) -> Result<LoadedSession, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc: SessionFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let session_id = Uuid::parse_str(&doc.session_id).map_err(|e| e.to_string())?;
    let mut messages = Vec::new();
    for m in doc.messages {
        match m.role.as_str() {
            "user" => messages.push(TuiMessage::User { content: m.content }),
            "assistant" => messages.push(TuiMessage::Assistant {
                content: m.content,
                complete: true,
            }),
            _ => {}
        }
    }
    let started_at = DateTime::parse_from_rfc3339(&doc.started_at)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(LoadedSession {
        session_id,
        project_root: PathBuf::from(doc.project_root),
        model_name: doc.model,
        started_at,
        messages,
    })
}

/// Returns `~/.akmon/sessions` when `HOME` / `USERPROFILE` is set.
pub fn sessions_directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(sessions_directory_under_home(&home))
}

/// Joins `.akmon/sessions` under `home` (used by tests and [`sessions_directory`]).
pub fn sessions_directory_under_home(home: &std::path::Path) -> PathBuf {
    home.join(".akmon").join("sessions")
}

/// Writes `app` + `config` to `~/.akmon/sessions/{session_id}.json`.
///
/// When `dir_override` is [`Some`], that directory is used instead of resolving `HOME`.
pub fn save_session_snapshot(
    app: &TuiApp,
    config: &TuiLaunchConfig,
    started_at: DateTime<Utc>,
    dir_override: Option<&std::path::Path>,
) -> io::Result<PathBuf> {
    let dir = match dir_override {
        Some(d) => d.to_path_buf(),
        None => sessions_directory().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "HOME or USERPROFILE not set; cannot save session",
            )
        })?,
    };
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", config.session_id));
    let messages = transcript_messages(app);
    let doc = PersistedSession {
        session_id: config.session_id.to_string(),
        project_root: config.project_root.display().to_string(),
        model: config.model_name.clone(),
        started_at: started_at.to_rfc3339(),
        messages,
        total_input_tokens: app.total_input_tokens,
        total_cache_read_tokens: app.total_cache_read_tokens,
        total_output_tokens: app.total_output_tokens,
    };
    let json = serde_json::to_string_pretty(&doc).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("session json: {e}"))
    })?;
    fs::write(&path, json)?;
    Ok(path)
}

/// Expected snapshot path for tests when `HOME` is set.
pub fn session_file_path_for(session_id: Uuid) -> Option<PathBuf> {
    Some(sessions_directory()?.join(format!("{session_id}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn sessions_directory_under_home_shape() {
        let p = sessions_directory_under_home(Path::new("/tmp/fakehome"));
        assert!(p.ends_with(".akmon/sessions"));
        assert!(p.starts_with("/tmp/fakehome"));
    }

    #[test]
    fn save_session_snapshot_writes_uuid_json() {
        let dir = tempdir().expect("tempdir");
        let sessions = dir.path().join("sessions");
        let app = crate::TuiApp::new(crate::TuiLaunchConfig {
            version: "t".into(),
            project_root: Path::new("/p").to_path_buf(),
            model_name: "m".into(),
            mode_label: "INTERACTIVE".into(),
            session_id: uuid::Uuid::nil(),
            max_iterations: 5,
            index_enabled: false,
            anthropic_key: None,
            openrouter_key: None,
            openai_key: None,
            groq_key: None,
            azure_endpoint: None,
            azure_key: None,
            azure_api_version: "2024-02-01".into(),
            bedrock: false,
            aws_region: "us-east-1".into(),
            openai_compatible_url: None,
            openai_compatible_key: None,
            ollama_url: "http://x".into(),
            shell_allow: Vec::new(),
            web_fetch: false,
            yes_web: false,
            auto_yes: false,
            mcp_servers: Vec::new(),
            audit_log_path: Path::new("/a").to_path_buf(),
            akmon_md: None,
            has_akmon_md: false,
            sandbox_has_git_root: true,
            semantic_index: None,
            auto_commit: false,
            planner_model: "llama3.2".into(),
        });
        let cfg = crate::TuiLaunchConfig {
            version: "t".into(),
            project_root: Path::new("/p").to_path_buf(),
            model_name: "m".into(),
            mode_label: "INTERACTIVE".into(),
            session_id: uuid::Uuid::nil(),
            max_iterations: 5,
            index_enabled: false,
            anthropic_key: None,
            openrouter_key: None,
            openai_key: None,
            groq_key: None,
            azure_endpoint: None,
            azure_key: None,
            azure_api_version: "2024-02-01".into(),
            bedrock: false,
            aws_region: "us-east-1".into(),
            openai_compatible_url: None,
            openai_compatible_key: None,
            ollama_url: "http://x".into(),
            shell_allow: Vec::new(),
            web_fetch: false,
            yes_web: false,
            auto_yes: false,
            mcp_servers: Vec::new(),
            audit_log_path: Path::new("/a").to_path_buf(),
            akmon_md: None,
            has_akmon_md: false,
            sandbox_has_git_root: true,
            semantic_index: None,
            auto_commit: false,
            planner_model: "llama3.2".into(),
        };
        let path = save_session_snapshot(&app, &cfg, Utc::now(), Some(&sessions)).expect("save");
        assert_eq!(
            path,
            sessions.join("00000000-0000-0000-0000-000000000000.json")
        );
        let raw = std::fs::read_to_string(&path).expect("read");
        assert!(raw.contains("\"session_id\""));
        assert!(raw.contains("00000000-0000-0000-0000-000000000000"));
    }
}

fn transcript_messages(app: &TuiApp) -> Vec<PersistedMessage> {
    let mut out = Vec::new();
    for m in &app.messages {
        match m {
            TuiMessage::User { content } => out.push(PersistedMessage {
                role: "user".into(),
                content: content.clone(),
            }),
            TuiMessage::Assistant { content, complete } => {
                if *complete {
                    out.push(PersistedMessage {
                        role: "assistant".into(),
                        content: content.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}
