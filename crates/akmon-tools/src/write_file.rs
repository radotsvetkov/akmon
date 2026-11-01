//! Write a UTF-8 file inside the sandbox using an atomic rename.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;

use crate::Tool;
use crate::atomic_write::atomic_write_in_sandbox;
use crate::context::ToolContext;
use crate::diff_render::unified_diff_text;
use crate::file_change_set::{ChangeSetMode, FileChange, FileChangeSet, diff_stats_from_unified};
use crate::output::{ToolErrorCode, ToolOutput};

fn write_file_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::WriteFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Writes or overwrites a UTF-8 text file atomically (temp file + rename in the target directory).
///
/// The sandbox resolves the **parent** directory of `path` (the final path segment must be a normal file name), because [`akmon_core::Sandbox::resolve`] requires the resolved path to exist; the parent directory must already exist.
///
/// A final-component symlink is followed only when its target also resolves inside the
/// sandbox (the real file is then read and replaced); a symlink pointing outside the sandbox,
/// or a dangling one, is refused **before** anything is read, so outside contents can never
/// leak into the returned diff (even with `dry_run`).
pub struct WriteFileTool;

impl WriteFileTool {
    /// Creates a new write-file tool instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Sandbox-relative path for success payloads (forward slashes).
fn relative_path_display(file: &Path, sandbox_root: &Path) -> Option<String> {
    let c = std::fs::canonicalize(file).ok()?;
    let root = std::fs::canonicalize(sandbox_root).ok()?;
    c.strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// Splits a sandbox-relative path into a parent segment for [`akmon_core::Sandbox::resolve`] and a single final file name.
///
/// The last component must be a normal name (not `.`, `..`, or a root), so `join(file_name)` cannot climb past the resolved parent.
fn split_write_path(path_str: &str) -> Result<(&str, &str), ToolOutput> {
    let path = Path::new(path_str);
    match path.components().next_back() {
        Some(Component::Normal(os)) => {
            let file_name = match os.to_str() {
                Some(s) if !s.is_empty() => s,
                _ => {
                    return Err(ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: "path file name is not valid UTF-8".into(),
                    });
                }
            };
            let parent_str = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => match p.to_str() {
                    Some(s) => s,
                    None => {
                        return Err(ToolOutput::Error {
                            code: ToolErrorCode::InvalidArgs,
                            message: "path parent is not valid UTF-8".into(),
                        });
                    }
                },
                _ => ".",
            };
            Ok((parent_str, file_name))
        }
        _ => Err(ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: "path must end with a single file name (not \".\", \"..\", or a root)".into(),
        }),
    }
}

/// Resolves the final path component under the already-sandboxed `resolved_parent`.
///
/// - missing entry: returned as-is (new file);
/// - regular file: returned as-is;
/// - symlink: its target must resolve inside the sandbox, and the canonical target is
///   returned so the real file is read and atomically replaced; an outside or dangling
///   target is refused without reading it;
/// - anything else (directory, socket, ...): refused.
async fn resolve_final_component(
    ctx: &ToolContext,
    resolved_parent: &Path,
    file_name: &str,
    path_str: &str,
) -> Result<PathBuf, ToolOutput> {
    let candidate = resolved_parent.join(file_name);
    let meta = match fs::symlink_metadata(&candidate).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
        Err(e) => {
            return Err(ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("could not stat path: {e}"),
            });
        }
    };
    let target = if meta.file_type().is_symlink() {
        match ctx.resolve_path(&candidate) {
            Ok(t) => t,
            Err(SandboxError::PathEscape { .. }) => {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::PathEscape,
                    message: format!("symlink target escapes sandbox: {path_str}"),
                });
            }
            Err(SandboxError::Canonicalize(_)) => {
                return Err(ToolOutput::Error {
                    code: ToolErrorCode::PathEscape,
                    message: format!(
                        "refusing to write through a dangling or unresolvable symlink: {path_str}"
                    ),
                });
            }
        }
    } else {
        candidate
    };
    match fs::metadata(&target).await {
        Ok(m) if !m.is_file() => Err(ToolOutput::Error {
            code: ToolErrorCode::NotAFile,
            message: format!("not a regular file: {path_str}"),
        }),
        _ => Ok(target),
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Writes UTF-8 text to a file inside the sandbox using an atomic replace (no torn writes)."
    }

    fn required_permissions(&self) -> &[Permission] {
        write_file_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to write within the project sandbox"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "dry_run": {
                    "type": "boolean",
                    "default": false,
                    "description": "Validate and generate diff payload without writing file changes."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty \"path\" string".into(),
                };
            }
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing \"content\" string".into(),
                };
            }
        };
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let bytes = content.as_bytes();

        let (parent_str, file_name) = match split_write_path(path_str) {
            Ok(pair) => pair,
            Err(out) => return out,
        };

        let resolved_parent = match ctx.resolve_path(parent_str) {
            Ok(p) => p,
            Err(SandboxError::PathEscape { .. }) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PathEscape,
                    message: format!("path escapes sandbox: {path_str}"),
                };
            }
            Err(SandboxError::Canonicalize(e)) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return ToolOutput::Error {
                        code: ToolErrorCode::NotFound,
                        message: format!("parent path does not exist: {parent_str}"),
                    };
                }
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("could not resolve parent path: {e}"),
                };
            }
        };

        let resolved =
            match resolve_final_component(ctx, &resolved_parent, file_name, path_str).await {
                Ok(p) => p,
                Err(out) => return out,
            };
        let old_content = match fs::read(&resolved).await {
            Ok(existing) => std::str::from_utf8(&existing).ok().map(ToOwned::to_owned),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => None,
        };

        if !dry_run {
            match atomic_write_in_sandbox(ctx, &resolved, bytes).await {
                Ok(_) => {}
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("write failed: {e}"),
                    };
                }
            }
        }

        let rel = relative_path_display(&resolved, ctx.primary_root().as_ref())
            .unwrap_or_else(|| path_str.to_string().replace('\\', "/"));
        let before = old_content.as_deref().unwrap_or("");
        let unified = unified_diff_text(before, content, rel.as_str());
        let (lines_added, lines_removed, lines_changed) = diff_stats_from_unified(&unified);
        let mode = if dry_run {
            ChangeSetMode::DryRun
        } else {
            ChangeSetMode::Applied
        };
        let payload = FileChangeSet::from_files(
            mode,
            vec![FileChange {
                path: rel,
                diff: unified,
                lines_added,
                lines_removed,
                lines_changed,
            }],
        );
        match serde_json::to_string(&payload) {
            Ok(content) => ToolOutput::Success { content },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("serialize write result: {e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use serde_json::json;

    use crate::read_file::ReadFileTool;

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext::new(
            Sandbox::new(root),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let w = WriteFileTool::new();
        let r = ReadFileTool::new();
        let c = ctx(dir.path());
        let out = w
            .execute(json!({ "path": "out.txt", "content": "payload" }), &c)
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: serde_json::Value = serde_json::from_str(&content).expect("json");
        assert_eq!(v["type"], "file_change_set");
        assert_eq!(v["mode"], "applied");
        let read = r.execute(json!({ "path": "out.txt" }), &c).await;
        assert_eq!(
            read,
            ToolOutput::Success {
                content: "payload".into(),
            }
        );
    }

    #[tokio::test]
    async fn write_path_escape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inside = dir.path().join("inside");
        tokio::fs::create_dir_all(&inside).await.expect("mkdir");
        let w = WriteFileTool::new();
        let out = w
            .execute(json!({ "path": "../x.txt", "content": "a" }), &ctx(&inside))
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::PathEscape,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn write_invalid_args_missing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let w = WriteFileTool::new();
        let out = w
            .execute(json!({ "content": "only" }), &ctx(dir.path()))
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                ..
            }
        ));
    }

    /// AGT-02 regression: a final-component symlink to an outside file must be refused
    /// before reading, so the outside content never reaches the diff (dry run included).
    #[cfg(unix)]
    #[tokio::test]
    async fn outside_symlink_is_refused_without_leaking_contents() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let secret = outside.join("credentials");
        std::fs::write(&secret, "AWS_SECRET=hunter2\n").expect("w");
        symlink(&secret, repo.join("src/notes.txt")).expect("symlink");
        let w = WriteFileTool::new();
        for dry_run in [true, false] {
            let out = w
                .execute(
                    json!({ "path": "src/notes.txt", "content": "x", "dry_run": dry_run }),
                    &ctx(&repo),
                )
                .await;
            let ToolOutput::Error { code, message } = out else {
                panic!("expected refusal, got {out:?}");
            };
            assert_eq!(code, ToolErrorCode::PathEscape);
            assert!(!message.contains("hunter2"));
        }
        assert_eq!(
            std::fs::read_to_string(&secret).expect("r"),
            "AWS_SECRET=hunter2\n"
        );
        assert!(
            std::fs::symlink_metadata(repo.join("src/notes.txt"))
                .expect("meta")
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dangling_symlink_is_refused() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        symlink(dir.path().join("missing"), dir.path().join("link.txt")).expect("symlink");
        let out = WriteFileTool::new()
            .execute(
                json!({ "path": "link.txt", "content": "x" }),
                &ctx(dir.path()),
            )
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::PathEscape,
                ..
            }
        ));
        assert!(!dir.path().join("missing").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inside_symlink_replaces_real_target_and_keeps_link() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("real.txt"), "old\n").expect("w");
        symlink(dir.path().join("real.txt"), dir.path().join("alias.txt")).expect("symlink");
        let out = WriteFileTool::new()
            .execute(
                json!({ "path": "alias.txt", "content": "new\n" }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success, got {out:?}");
        };
        assert!(content.contains("-old") && content.contains("+new"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("real.txt")).expect("r"),
            "new\n"
        );
        assert!(
            std::fs::symlink_metadata(dir.path().join("alias.txt"))
                .expect("meta")
                .file_type()
                .is_symlink()
        );
    }

    #[tokio::test]
    async fn directory_target_is_not_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("d")).expect("mkdir");
        let out = WriteFileTool::new()
            .execute(json!({ "path": "d", "content": "x" }), &ctx(dir.path()))
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::NotAFile,
                ..
            }
        ));
    }

    /// AGT-01 regression through the tool surface: planted temp-name symlinks are ignored.
    #[cfg(unix)]
    #[tokio::test]
    async fn planted_temp_symlink_does_not_redirect_write() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");
        let victim = outside.join("bashrc");
        std::fs::write(&victim, "safe\n").expect("w");
        for n in 0..16 {
            symlink(&victim, repo.join(format!("src/.akmon-w-{n}-main.rs"))).expect("symlink");
        }
        let out = WriteFileTool::new()
            .execute(
                json!({ "path": "src/main.rs", "content": "pwned\n" }),
                &ctx(&repo),
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }), "{out:?}");
        assert_eq!(std::fs::read_to_string(&victim).expect("r"), "safe\n");
        assert_eq!(
            std::fs::read_to_string(repo.join("src/main.rs")).expect("r"),
            "pwned\n"
        );
    }

    #[tokio::test]
    async fn dry_run_does_not_write_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let w = WriteFileTool::new();
        let out = w
            .execute(
                json!({ "path": "out.txt", "content": "payload", "dry_run": true }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: serde_json::Value = serde_json::from_str(&content).expect("json");
        assert_eq!(v["mode"], "dry_run");
        assert!(!dir.path().join("out.txt").exists());
    }
}
