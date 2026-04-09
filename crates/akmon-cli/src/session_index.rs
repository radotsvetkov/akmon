//! Last session id per project path (`~/.akmon/last_session.json`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Canonical JSON index: one row per sandbox root.
#[derive(Serialize, Deserialize, Default)]
pub struct SessionIndex {
    /// Maps canonical project path → last session for that path.
    pub by_path: HashMap<String, SessionEntry>,
}

/// One row stored for a project directory.
#[derive(Serialize, Deserialize, Clone)]
pub struct SessionEntry {
    /// Full session UUID string.
    pub session_id: String,
    pub model: String,
    pub started_at: String,
    pub turn_count: u32,
}

impl SessionIndex {
    /// Path to the JSON file (`~/.akmon/last_session.json`).
    #[must_use]
    pub fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".akmon/last_session.json")
    }

    /// Loads the index, or empty on missing/invalid file.
    #[must_use]
    pub fn load() -> Self {
        let path = Self::path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Writes the index best-effort (ignores most errors).
    pub fn save(&self) {
        if let Some(parent) = Self::path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), json);
        }
    }

    /// Inserts or replaces the entry for `project_root` and persists.
    pub fn record(&mut self, project_root: &Path, entry: SessionEntry) {
        let key = canonical_key(project_root);
        self.by_path.insert(key, entry);
        self.save();
    }

    /// Looks up the last session for this project path.
    #[must_use]
    pub fn get_for_project(&self, project_root: &Path) -> Option<&SessionEntry> {
        let key = canonical_key(project_root);
        self.by_path.get(&key)
    }
}

/// Stable filesystem key for the index (matches [`SessionIndex::record`]).
#[must_use]
pub fn canonical_key(path: &Path) -> String {
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}
