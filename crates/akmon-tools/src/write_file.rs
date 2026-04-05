//! Write a UTF-8 file inside the sandbox using an atomic rename.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;

use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};
use crate::Tool;

static WRITE_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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
/// The sandbox resolves the **parent** directory of `path` (the final path segment must be a normal file name), because [`akmon_core::Sandbox::resolve`] requires the resolved path to exist; missing parents are created before the atomic write.
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

/// Writes `content` to `path` via a unique temp file in the same directory, then renames into place.
pub(crate) async fn atomic_write_utf8(path: &Path, content: &[u8]) -> std::io::Result<usize> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no parent directory",
        )
    })?;

    fs::create_dir_all(parent).await?;

    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no file name",
        )
    })?;

    let n = WRITE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".akmon-w-{n}-{}", file_name.to_string_lossy());
    let tmp_path = parent.join(tmp_name);

    fs::write(&tmp_path, content).await?;
    fs::rename(&tmp_path, path).await?;
    Ok(content.len())
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

        let resolved = resolved_parent.join(file_name);

        match atomic_write_utf8(&resolved, bytes).await {
            Ok(n) => ToolOutput::Success {
                content: format!("wrote {n} bytes to {}", resolved.display()),
            },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("write failed: {e}"),
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
            .execute(
                json!({ "path": "out.txt", "content": "payload" }),
                &c,
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }));
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
            .execute(
                json!({ "path": "../x.txt", "content": "a" }),
                &ctx(&inside),
            )
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
            .execute(
                json!({ "content": "only" }),
                &ctx(dir.path()),
            )
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                ..
            }
        ));
    }
}
