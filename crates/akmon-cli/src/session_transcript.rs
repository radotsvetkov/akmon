//! Load/save `~/.akmon/sessions/{session_id}.json` in the same shape as the TUI snapshot.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use akmon_models::{Message, MessageRole};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct FileMsg {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    session_id: String,
    project_root: String,
    model: String,
    started_at: String,
    messages: Vec<FileMsg>,
    total_input_tokens: u32,
    total_cache_read_tokens: u32,
    total_output_tokens: u32,
}

fn sessions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".akmon").join("sessions"))
}

/// Arguments for [`save_headless_session_file`].
pub struct HeadlessSessionSnapshot<'a> {
    pub session_id: Uuid,
    pub project_root: &'a Path,
    pub model: &'a str,
    pub messages: &'a [Message],
    pub started_at_rfc3339: &'a str,
    pub total_input_tokens: u32,
    pub total_cache_read_tokens: u32,
    pub total_output_tokens: u32,
}

/// Saves transcript compatible with the TUI session picker (`load_session_summaries`).
pub fn save_headless_session_file(snap: HeadlessSessionSnapshot<'_>) -> io::Result<PathBuf> {
    let dir = sessions_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "HOME/USERPROFILE not set; cannot save session",
        )
    })?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", snap.session_id));
    let mut rows: Vec<FileMsg> = Vec::new();
    for m in snap.messages {
        let role = match m.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            _ => continue,
        };
        rows.push(FileMsg {
            role: role.to_string(),
            content: m.content.clone(),
        });
    }
    let doc = SessionFile {
        session_id: snap.session_id.to_string(),
        project_root: snap.project_root.display().to_string(),
        model: snap.model.into(),
        started_at: snap.started_at_rfc3339.into(),
        messages: rows,
        total_input_tokens: snap.total_input_tokens,
        total_cache_read_tokens: snap.total_cache_read_tokens,
        total_output_tokens: snap.total_output_tokens,
    };
    let json = serde_json::to_string_pretty(&doc)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(&path, json)?;
    Ok(path)
}

/// Loads model [`Message`] rows when the file exists and `project_root` matches.
pub fn load_resume_messages(session_id: Uuid, project_root: &Path) -> Result<Vec<Message>, String> {
    let Some(dir) = sessions_dir() else {
        return Err("HOME not set".into());
    };
    let path = dir.join(format!("{session_id}.json"));
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let doc: SessionFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let canon = crate::session_index::canonical_key(project_root);
    let file_root = crate::session_index::canonical_key(Path::new(&doc.project_root));
    if canon != file_root {
        return Err(format!(
            "session file project_root mismatch (expected {}, file has {})",
            canon, file_root
        ));
    }
    let mut out = Vec::new();
    for m in doc.messages {
        let role = match m.role.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            _ => continue,
        };
        out.push(Message {
            role,
            content: m.content,
        });
    }
    Ok(out)
}

/// Resolves `input` as a full UUID or a unique prefix among `~/.akmon/sessions/*.json` basenames.
pub fn resolve_session_id_from_cli_arg(input: &str) -> Result<Uuid, String> {
    let needle = input.trim();
    if needle.is_empty() {
        return Err("empty session id".into());
    }
    if let Ok(u) = Uuid::parse_str(needle) {
        return Ok(u);
    }
    let Some(dir) = sessions_dir() else {
        return Err("HOME not set".into());
    };
    let Ok(rd) = fs::read_dir(&dir) else {
        return Err("cannot read ~/.akmon/sessions".into());
    };
    let mut matches: Vec<Uuid> = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id_part) = name.strip_suffix(".json") else {
            continue;
        };
        if id_part.starts_with(needle) && let Ok(u) = Uuid::parse_str(id_part) {
            matches.push(u);
        }
    }
    match matches.len() {
        0 => Err(format!("no session file matches prefix `{needle}`")),
        1 => Ok(matches[0]),
        _ => Err(format!(
            "ambiguous session prefix `{needle}` — use a longer prefix or full UUID"
        )),
    }
}
