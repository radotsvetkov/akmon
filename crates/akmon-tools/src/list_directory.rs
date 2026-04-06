//! List files and directories inside the sandbox (read-only).

use std::path::PathBuf;
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

fn list_directory_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::ReadFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Lists immediate children of a sandbox-relative directory as sorted JSON (`entries` with `name` and `kind`).
pub struct ListDirectoryTool;

impl ListDirectoryTool {
    /// Creates a new list-directory tool.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListDirectoryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories at a path within the project sandbox. Use this before reading files to find correct paths."
    }

    fn required_permissions(&self) -> &[Permission] {
        list_directory_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to list. Use '.' for project root."
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
                    message: format!("failed to resolve path: {e}"),
                };
            }
        };

        let meta = match fs::metadata(&resolved).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return ToolOutput::Error {
                    code: ToolErrorCode::NotFound,
                    message: format!("path not found: {path_str}"),
                };
            }
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("cannot stat path: {e}"),
                };
            }
        };

        if !meta.is_dir() {
            return ToolOutput::Error {
                code: ToolErrorCode::NotAFile,
                message: format!("path is not a directory: {path_str}"),
            };
        }

        let mut rd = match fs::read_dir(&resolved).await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("cannot read directory: {e}"),
                };
            }
        };

        let mut rows: Vec<(String, String)> = Vec::new();
        loop {
            match rd.next_entry().await {
                Ok(None) => break,
                Ok(Some(ent)) => {
                    let name = ent.file_name().to_string_lossy().into_owned();
                    let p = ent.path();
                    let kind = match fs::metadata(&p).await {
                        Ok(m) => {
                            if m.is_dir() {
                                "dir"
                            } else {
                                "file"
                            }
                        }
                        Err(_) => "file",
                    };
                    rows.push((name, kind.to_string()));
                }
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("directory read error: {e}"),
                    };
                }
            }
        }

        rows.sort_by(|a, b| a.0.cmp(&b.0));

        let entries: Vec<JsonValue> = rows
            .into_iter()
            .map(|(name, kind)| {
                serde_json::json!({
                    "name": name,
                    "kind": kind,
                })
            })
            .collect();

        let payload = serde_json::json!({
            "path": path_str,
            "entries": entries,
        });

        match serde_json::to_string_pretty(&payload) {
            Ok(s) => ToolOutput::Success { content: s },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("failed to serialize listing: {e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use tempfile::tempdir;

    fn ctx_for_sandbox(root: &std::path::Path) -> ToolContext {
        ToolContext::new(
            Sandbox::new(root),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn list_directory_returns_entries() {
        let tmp = tempdir().expect("tmp");
        fs::write(tmp.path().join("a.txt"), b"x")
            .await
            .expect("write");
        fs::create_dir(tmp.path().join("sub")).await.expect("dir");
        let tool = ListDirectoryTool::new();
        let ctx = ctx_for_sandbox(tmp.path());
        let out = tool.execute(serde_json::json!({ "path": "." }), &ctx).await;
        match out {
            ToolOutput::Success { content } => {
                let v: JsonValue = serde_json::from_str(&content).expect("json");
                assert_eq!(v["path"], ".");
                let entries = v["entries"].as_array().expect("entries");
                let names: Vec<&str> = entries
                    .iter()
                    .map(|e| e["name"].as_str().expect("name"))
                    .collect();
                assert!(names.contains(&"a.txt"));
                assert!(names.contains(&"sub"));
            }
            ToolOutput::Error { message, .. } => panic!("unexpected error: {message}"),
        }
    }

    #[tokio::test]
    async fn list_directory_path_escape() {
        let dir = tempdir().expect("tmp");
        let inside = dir.path().join("inside");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&inside).await.expect("mkdir");
        fs::create_dir_all(&outside).await.expect("mkdir");
        let tool = ListDirectoryTool::new();
        let ctx = ctx_for_sandbox(&inside);
        let out = tool
            .execute(serde_json::json!({ "path": "../outside" }), &ctx)
            .await;
        match out {
            ToolOutput::Error {
                code: ToolErrorCode::PathEscape,
                ..
            } => {}
            other => panic!("expected PathEscape, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_directory_not_found() {
        let tmp = tempdir().expect("tmp");
        let tool = ListDirectoryTool::new();
        let ctx = ctx_for_sandbox(tmp.path());
        let out = tool
            .execute(serde_json::json!({ "path": "nope" }), &ctx)
            .await;
        match out {
            ToolOutput::Error {
                code: ToolErrorCode::NotFound,
                ..
            } => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_directory_not_a_dir() {
        let tmp = tempdir().expect("tmp");
        let f = tmp.path().join("only_file");
        fs::write(&f, b"hi").await.expect("write");
        let tool = ListDirectoryTool::new();
        let ctx = ctx_for_sandbox(tmp.path());
        let out = tool
            .execute(serde_json::json!({ "path": "only_file" }), &ctx)
            .await;
        match out {
            ToolOutput::Error {
                code: ToolErrorCode::NotAFile,
                ..
            } => {}
            other => panic!("expected NotAFile, got {other:?}"),
        }
    }
}
