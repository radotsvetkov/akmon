//! Filesystem sandbox: canonical paths and repository-root boundary checks.

use std::path::{Path, PathBuf};

use dunce::canonicalize;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Violations and I/O failures while resolving a path inside a sandbox root.
#[derive(Debug, Error)]
pub enum SandboxError {
    /// Canonicalization failed (missing path, I/O error, etc.).
    #[error("failed to canonicalize path: {0}")]
    Canonicalize(#[from] std::io::Error),

    /// Resolved path is outside all allowed roots after canonicalization.
    #[error(
        "path escapes sandbox: attempted `{}`, not within boundary `{}`",
        attempted.display(),
        boundary.display()
    )]
    PathEscape {
        /// Path that was rejected.
        attempted: PathBuf,
        /// Primary sandbox root (shown for error messages).
        boundary: PathBuf,
    },
}

/// Filesystem boundary: one primary root plus optional additional roots
/// (e.g. declared monorepo siblings). Every [`Sandbox::resolve`] result is
/// canonical and must fall under at least one allowed root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sandbox {
    primary_root: PathBuf,
    /// Extra roots explicitly allowed by configuration; never auto-expanded.
    additional_roots: Vec<PathBuf>,
}

impl Sandbox {
    /// Creates a sandbox anchored at `primary_root` (should exist for
    /// canonicalization to succeed on first resolve; callers may create dirs first).
    pub fn new(primary_root: impl Into<PathBuf>) -> Self {
        Self {
            primary_root: primary_root.into(),
            additional_roots: Vec::new(),
        }
    }

    /// Same as [`Sandbox::new`] but registers extra allowed roots (each checked
    /// independently in [`Sandbox::resolve`]).
    pub fn with_additional_roots(
        primary_root: impl Into<PathBuf>,
        additional_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            primary_root: primary_root.into(),
            additional_roots,
        }
    }

    /// Returns the configured primary root without canonicalization.
    pub fn primary_root(&self) -> &Path {
        &self.primary_root
    }

    /// Canonicalizes `user_path` and ensures it lies under the primary root or
    /// one of the additional roots after **fully** resolving symlinks.
    ///
    /// Relative paths are interpreted relative to the primary root.
    pub fn resolve(&self, user_path: impl AsRef<Path>) -> Result<PathBuf, SandboxError> {
        let user_path = user_path.as_ref();
        let joined = if user_path.is_absolute() {
            user_path.to_path_buf()
        } else {
            self.primary_root.join(user_path)
        };

        let resolved = canonicalize(&joined)?;
        let roots = self.canonical_roots()?;

        for root in &roots {
            if path_is_within_root(root, &resolved) {
                return Ok(resolved);
            }
        }

        Err(SandboxError::PathEscape {
            attempted: resolved,
            boundary: roots
                .first()
                .cloned()
                .unwrap_or_else(|| self.primary_root.clone()),
        })
    }

    fn canonical_roots(&self) -> Result<Vec<PathBuf>, SandboxError> {
        let mut roots = Vec::with_capacity(1 + self.additional_roots.len());
        roots.push(canonicalize(&self.primary_root)?);
        for extra in &self.additional_roots {
            roots.push(canonicalize(extra)?);
        }
        Ok(roots)
    }
}

/// `candidate` is inside `root` if `root` is a prefix in terms of path components.
fn path_is_within_root(root: &Path, candidate: &Path) -> bool {
    candidate.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn traversal_outside_sandbox_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("repo");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&inner).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let sandbox = Sandbox::new(&inner);

        let err = sandbox.resolve("../outside").unwrap_err();
        match err {
            SandboxError::PathEscape { .. } => {}
            other => panic!("expected PathEscape, got {other:?}"),
        }
    }

    #[test]
    fn symlink_escape_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "nope").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = repo.join("escape");
            symlink(&outside, &link).unwrap();
            let sandbox = Sandbox::new(&repo);
            let err = sandbox.resolve("escape/secret.txt").unwrap_err();
            assert!(
                matches!(err, SandboxError::PathEscape { .. }),
                "expected PathEscape, got {err:?}"
            );
        }
    }

    #[test]
    fn file_inside_repo_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("file.txt");
        fs::write(&f, "ok").unwrap();
        let sandbox = Sandbox::new(tmp.path());
        let got = sandbox.resolve("file.txt").unwrap();
        assert_eq!(got, dunce::canonicalize(&f).unwrap());
    }
}
