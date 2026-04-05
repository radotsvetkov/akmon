//! Typed permission requests evaluated by the policy engine.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A single capability the agent (or a tool) wants to exercise.
///
/// Each variant carries the parameters needed for policy checks and audit logs.
/// Paths are **logical** request paths; the [`crate::Sandbox`] still validates
/// them against allowed roots before any filesystem access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "permission", rename_all = "snake_case")]
pub enum Permission {
    /// Read bytes from a file at `path`.
    ReadFile {
        /// Target path (absolute or relative to sandbox primary root).
        path: PathBuf,
    },
    /// List directory entries at `path` (read-only directory listing).
    ListDirectory {
        /// Directory path (absolute or relative to sandbox primary root).
        path: PathBuf,
    },
    /// Create or modify a file at `path`.
    WriteFile {
        /// Target path (absolute or relative to sandbox primary root).
        path: PathBuf,
    },
    /// Run a shell command in `cwd` (PTY or subprocess — execution layer TBD).
    ExecuteCommand {
        /// Full command line as requested (policy may parse or restrict further).
        command: String,
        /// Working directory for the process.
        cwd: PathBuf,
    },
    /// Perform an HTTP fetch to `url` (domain allowlists applied by policy).
    NetworkFetch {
        /// Fully qualified URL string.
        url: String,
    },
}
