//! Read a UTF-8 text file inside the sandbox.

use std::path::PathBuf;
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// Default maximum bytes read to cap memory and token blow-up (1 MiB).
pub const DEFAULT_MAX_READ_BYTES: usize = 1024 * 1024;

fn read_file_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::ReadFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Reads a single text file from disk after sandbox resolution.
pub struct ReadFileTool {
    /// Maximum number of bytes to read from the file.
    max_bytes: usize,
}

impl ReadFileTool {
    /// Builds a reader with [`DEFAULT_MAX_READ_BYTES`].
    pub fn new() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_READ_BYTES,
        }
    }

    /// Overrides the read-size cap.
    pub fn with_max_bytes(mut self, n: usize) -> Self {
        self.max_bytes = n;
        self
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Reads the full contents of a UTF-8 text file at a path inside the project sandbox."
    }

    fn required_permissions(&self) -> &[Permission] {
        read_file_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the file within the project sandbox"
                }
            },
            "required": ["path"]
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

        let resolved = match ctx.resolve_path(path_str) {
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
                        message: format!("path not found: {path_str}"),
                    };
                }
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("could not resolve path: {e}"),
                };
            }
        };

        let meta = match fs::metadata(&resolved).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return ToolOutput::Error {
                    code: ToolErrorCode::NotFound,
                    message: format!("file not found: {}", resolved.display()),
                };
            }
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("metadata: {e}"),
                };
            }
        };

        if meta.is_dir() {
            return ToolOutput::Error {
                code: ToolErrorCode::NotAFile,
                message: format!("not a regular file: {}", resolved.display()),
            };
        }

        if !meta.is_file() {
            return ToolOutput::Error {
                code: ToolErrorCode::NotAFile,
                message: format!("path is not a regular file: {}", resolved.display()),
            };
        }

        let len = meta.len() as usize;
        if len > self.max_bytes {
            return ToolOutput::Error {
                code: ToolErrorCode::TooLarge,
                message: format!("file size {len} exceeds limit {}", self.max_bytes),
            };
        }

        let file = match fs::File::open(&resolved).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return ToolOutput::Error {
                    code: ToolErrorCode::NotFound,
                    message: format!("file not found: {}", resolved.display()),
                };
            }
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("open failed: {e}"),
                };
            }
        };

        let mut buf = Vec::new();
        if let Err(e) = file
            .take(self.max_bytes as u64 + 1)
            .read_to_end(&mut buf)
            .await
        {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("read failed: {e}"),
            };
        }

        if buf.len() > self.max_bytes {
            return ToolOutput::Error {
                code: ToolErrorCode::TooLarge,
                message: format!("read exceeded limit {}", self.max_bytes),
            };
        }

        let text = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(_) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::BinaryContent,
                    message: "file is not valid UTF-8".into(),
                };
            }
        };

        ToolOutput::Success { content: text }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use serde_json::json;

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext::new(
            Sandbox::new(root),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn read_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("a.txt");
        tokio::fs::write(&p, b"hello").await.expect("write");
        let tool = ReadFileTool::new();
        let out = tool
            .execute(json!({ "path": "a.txt" }), &ctx(dir.path()))
            .await;
        assert_eq!(
            out,
            ToolOutput::Success {
                content: "hello".into(),
            }
        );
    }

    #[tokio::test]
    async fn read_path_escape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inside = dir.path().join("inside");
        let outside = dir.path().join("outside");
        tokio::fs::create_dir_all(&inside).await.expect("mkdir");
        tokio::fs::create_dir_all(&outside).await.expect("mkdir");
        let tool = ReadFileTool::new();
        let out = tool
            .execute(json!({ "path": "../outside" }), &ctx(&inside))
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
    async fn read_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = ReadFileTool::new();
        let out = tool
            .execute(json!({ "path": "nope.txt" }), &ctx(dir.path()))
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::NotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn read_not_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::create_dir_all(dir.path().join("sub"))
            .await
            .expect("mkdir");
        let tool = ReadFileTool::new();
        let out = tool
            .execute(json!({ "path": "sub" }), &ctx(dir.path()))
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::NotAFile,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn read_binary_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("b.bin");
        tokio::fs::write(&p, &[0xFFu8, 0xFEu8])
            .await
            .expect("write");
        let tool = ReadFileTool::new();
        let out = tool
            .execute(json!({ "path": "b.bin" }), &ctx(dir.path()))
            .await;
        assert!(matches!(
            out,
            ToolOutput::Error {
                code: ToolErrorCode::BinaryContent,
                ..
            }
        ));
    }
}
