//! Per-invocation environment shared by all tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use akmon_core::{PolicyEngine, Sandbox, SandboxError};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// First 12 hex chars of SHA-256 of the project root display string (stable per checkout path).
#[must_use]
pub fn project_hash_for_root(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.display().to_string().as_bytes());
    let full = format!("{:x}", hasher.finalize());
    full.chars().take(12).collect()
}

/// Execution context: sandbox path rules and the active policy engine (for future pre-flight checks).
///
/// Tools interact only through methods on this type; internals are not exposed.
pub struct ToolContext {
    sandbox: Sandbox,
    /// Held for upcoming policy pre-checks; not read in the file-tools slice.
    #[allow(dead_code)]
    policy: Arc<PolicyEngine>,
    session_id: Uuid,
    interactive: bool,
    project_hash: String,
}

impl ToolContext {
    /// Creates a context for tools running under `sandbox` and `policy`.
    pub fn new(sandbox: Sandbox, policy: Arc<PolicyEngine>) -> Self {
        let project_hash = project_hash_for_root(sandbox.primary_root());
        Self {
            sandbox,
            policy,
            session_id: Uuid::nil(),
            interactive: false,
            project_hash,
        }
    }

    /// Sets session id and whether the run can show interactive prompts (TUI vs headless).
    #[must_use]
    pub fn with_session(mut self, session_id: Uuid, interactive: bool) -> Self {
        self.session_id = session_id;
        self.interactive = interactive;
        self
    }

    /// When `false`, tools such as [`crate::AskFollowupTool`] must not block on user input.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Active session id (for per-session files such as todos).
    #[must_use]
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Short id for filenames (first 8 hex digits of the UUID simple form).
    #[must_use]
    pub fn session_id_short(&self) -> String {
        self.session_id.as_simple().to_string().chars().take(8).collect()
    }

    /// Stable hash for cross-session memory storage under `~/.akmon/memory/<hash>/`.
    #[must_use]
    pub fn project_hash(&self) -> &str {
        &self.project_hash
    }

    /// Resolves a user-supplied path to an absolute path inside the sandbox (see [`Sandbox::resolve`]).
    pub fn resolve_path(&self, path: impl AsRef<Path>) -> Result<std::path::PathBuf, SandboxError> {
        self.sandbox.resolve(path)
    }

    /// Project (sandbox) root directory; subprocess tools use this as `current_dir`.
    pub fn primary_root(&self) -> PathBuf {
        self.sandbox.primary_root().to_path_buf()
    }
}
