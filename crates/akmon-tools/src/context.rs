//! Per-invocation environment shared by all tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use akmon_core::{PolicyEngine, Sandbox, SandboxError};

/// Execution context: sandbox path rules and the active policy engine (for future pre-flight checks).
///
/// Tools interact only through methods on this type; internals are not exposed.
pub struct ToolContext {
    sandbox: Sandbox,
    /// Held for upcoming policy pre-checks; not read in the file-tools slice.
    #[allow(dead_code)]
    policy: Arc<PolicyEngine>,
}

impl ToolContext {
    /// Creates a context for tools running under `sandbox` and `policy`.
    pub fn new(sandbox: Sandbox, policy: Arc<PolicyEngine>) -> Self {
        Self { sandbox, policy }
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
