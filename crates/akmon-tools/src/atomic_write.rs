//! Sandbox-checked atomic file replacement shared by `write_file`, `edit` and `patch`.
//!
//! The content is written to a fresh temp file in the **same directory** as the target and
//! then renamed over it, so readers never observe a torn write. The temp file is hardened
//! against a planted-symlink attack (a malicious checkout shipping
//! `src/.akmon-w-0-main.rs -> ~/.bashrc`):
//!
//! - its name carries a random v4 UUID, so it cannot be predicted ahead of time;
//! - it is opened with `create_new` (`O_CREAT | O_EXCL`), which fails with `AlreadyExists`
//!   when *anything* (including a dangling or outside-pointing symlink) already occupies
//!   the name, so the open never follows a symlink;
//! - the parent directory is re-resolved through the [`akmon_core::Sandbox`] immediately
//!   before the write, and the canonical result is used for both the temp file and the
//!   rename target, so a parent swapped for an outside symlink is rejected.
//!
//! `rename(2)` replaces a symlink at the destination rather than writing through it.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use akmon_core::SandboxError;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::context::ToolContext;

/// Prefix of every in-flight temp file (tests look for leftovers by this prefix).
pub(crate) const TEMP_PREFIX: &str = ".akmon-w-";

/// Maximum bytes of the target file name embedded in the temp name (keeps us under NAME_MAX).
const TEMP_NAME_STEM_MAX: usize = 64;

/// Attempts before giving up when a random temp name is already taken.
const CREATE_ATTEMPTS: usize = 8;

fn invalid_input(msg: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidInput, msg.to_string())
}

/// Re-resolves `parent` through the sandbox and returns its canonical form.
fn canonical_parent_in_sandbox(ctx: &ToolContext, parent: &Path) -> std::io::Result<PathBuf> {
    match ctx.resolve_path(parent) {
        Ok(p) => Ok(p),
        Err(SandboxError::PathEscape { attempted, .. }) => Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "target directory escapes sandbox: {}",
                attempted.to_string_lossy()
            ),
        )),
        Err(SandboxError::Canonicalize(e)) => Err(e),
    }
}

/// Opens a brand-new temp file next to the target. Never follows or reuses an existing entry.
async fn create_temp_file(dir: &Path, stem: &str) -> std::io::Result<(PathBuf, fs::File)> {
    let mut last_err = None;
    for _ in 0..CREATE_ATTEMPTS {
        let tmp_path = dir.join(format!(
            "{TEMP_PREFIX}{}-{stem}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        match opts.open(&tmp_path).await {
            Ok(f) => return Ok((tmp_path, f)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last_err = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(ErrorKind::AlreadyExists, "could not create temp file")
    }))
}

async fn write_and_sync(file: &mut fs::File, content: &[u8]) -> std::io::Result<()> {
    file.write_all(content).await?;
    file.flush().await?;
    file.sync_all().await
}

/// Atomically replaces `path` with `content` after confirming its parent directory is inside
/// the sandbox of `ctx`.
///
/// `path` must name a file (its final component is used verbatim). The parent directory
/// must already exist. Returns the number of bytes written.
pub(crate) async fn atomic_write_in_sandbox(
    ctx: &ToolContext,
    path: &Path,
    content: &[u8],
) -> std::io::Result<usize> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_input("path has no parent directory"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| invalid_input("path has no file name"))?;

    let dir = canonical_parent_in_sandbox(ctx, parent)?;
    let target = dir.join(file_name);

    // Carry over the permission bits of an existing regular file (e.g. an executable script).
    let existing_perms = match fs::symlink_metadata(&target).await {
        Ok(m) if m.is_file() => Some(m.permissions()),
        _ => None,
    };

    let lossy = file_name.to_string_lossy();
    let stem = akmon_core::truncate_at_char_boundary(&lossy, TEMP_NAME_STEM_MAX);
    let (tmp_path, mut file) = create_temp_file(&dir, stem).await?;

    let written = async {
        write_and_sync(&mut file, content).await?;
        drop(file);
        if let Some(perms) = existing_perms {
            fs::set_permissions(&tmp_path, perms).await?;
        }
        fs::rename(&tmp_path, &target).await
    }
    .await;

    if let Err(e) = written {
        // Best effort: the temp file is ours (created with O_EXCL), so removing it is safe.
        let _ = fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    Ok(content.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};

    fn ctx(root: &Path) -> ToolContext {
        ToolContext::new(
            Sandbox::new(root),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    fn leftovers(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn writes_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let target = root.join("a.txt");
        let n = atomic_write_in_sandbox(&ctx(root), &target, b"hello")
            .await
            .expect("write");
        assert_eq!(n, 5);
        assert_eq!(std::fs::read_to_string(&target).expect("read"), "hello");
        assert!(leftovers(root).is_empty());
    }

    #[tokio::test]
    async fn rejects_parent_outside_sandbox() {
        let dir = tempfile::tempdir().expect("tmp");
        let repo = dir.path().join("repo");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&repo).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let err = atomic_write_in_sandbox(&ctx(&repo), &outside.join("x.txt"), b"nope")
            .await
            .expect_err("must reject");
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        assert!(!outside.join("x.txt").exists());
        assert!(leftovers(&outside).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn planted_temp_symlinks_are_never_followed() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tmp");
        let repo = dir.path().join("repo");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let victim = outside.join("bashrc");
        std::fs::write(&victim, "original").expect("w");
        // Plant links at every name the old counter-based scheme would have used.
        for n in 0..64 {
            symlink(
                &victim,
                repo.join("src").join(format!(".akmon-w-{n}-main.rs")),
            )
            .expect("symlink");
        }
        let target = repo.join("src").join("main.rs");
        atomic_write_in_sandbox(&ctx(&repo), &target, b"fn main() {}\n")
            .await
            .expect("write");
        assert_eq!(std::fs::read_to_string(&victim).expect("r"), "original");
        assert_eq!(
            std::fs::read_to_string(&target).expect("r"),
            "fn main() {}\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn preserves_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let target = root.join("run.sh");
        std::fs::write(&target, "#!/bin/sh\n").expect("w");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        atomic_write_in_sandbox(&ctx(root), &target, b"#!/bin/sh\necho hi\n")
            .await
            .expect("write");
        let mode = std::fs::metadata(&target)
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}
