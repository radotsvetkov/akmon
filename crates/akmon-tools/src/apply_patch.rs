//! Apply a unified diff to a single file when the model passes `file_path` + patch body (`apply_patch`).

use std::path::PathBuf;
use std::sync::OnceLock;

use akmon_core::Permission;
use async_trait::async_trait;
use serde_json::{Value as JsonValue, json};

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};
use crate::patch::{PatchTool, split_unified_patches};

fn apply_patch_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::WriteFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Prepends `---` / `+++` headers when the model sends only `@@` hunks; ensures a single-file diff.
fn normalize_single_file_patch(rel: &str, patch_body: &str) -> Result<String, ToolOutput> {
    let rel = rel.replace('\\', "/");
    let rel = rel.trim_start_matches('/').to_string();
    if rel.is_empty() {
        return Err(ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message: "\"file_path\" must not be empty".into(),
        });
    }

    let body = patch_body.replace('\r', "");
    let trimmed = body.trim_start();
    let full = if trimmed.starts_with("--- ") {
        body
    } else {
        format!("--- a/{rel}\n+++ b/{rel}\n{trimmed}")
    };

    let chunks = split_unified_patches(&full);
    if chunks.is_empty() {
        return Err(ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message:
                "patch must contain at least one hunk (or a full unified diff with --- headers)"
                    .into(),
        });
    }
    if chunks.len() > 1 {
        return Err(ToolOutput::Error {
            code: ToolErrorCode::InvalidArgs,
            message:
                "apply_patch applies to one file only; use the `patch` tool for multi-file diffs"
                    .into(),
        });
    }

    Ok(full)
}

/// Single-file unified diff helper: explicit path + patch text (like `git apply` for one file).
pub struct ApplyPatchTool;

impl ApplyPatchTool {
    /// Creates a new `apply_patch` tool instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ApplyPatchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to one file. More explicit than `patch` when the path and hunks are separate: you pass `file_path` plus the diff body (optionally without ---/+++ headers). For changing several files at once, use `patch`."
    }

    fn required_permissions(&self) -> &[Permission] {
        apply_patch_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Relative path to the file within the project sandbox"
                },
                "patch": {
                    "type": "string",
                    "description": "Unified diff: either a full hunk block (--- a/file, +++ b/file, @@ ...) or only the @@ hunks (headers are added from file_path)"
                },
                "dry_run": {
                    "type": "boolean",
                    "default": false,
                    "description": "Validate and generate diff payload without writing files."
                }
            },
            "required": ["file_path", "patch"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty \"file_path\" string".into(),
                };
            }
        };

        let patch_text = match args.get("patch").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty \"patch\" string".into(),
                };
            }
        };

        let full = match normalize_single_file_patch(path_str, patch_text) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        PatchTool::new()
            .execute(json!({ "patch": full, "dry_run": dry_run }), ctx)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext::new(
            Sandbox::new(root),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn apply_hunks_without_headers() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "hello\nworld\n").expect("w");
        let tool = ApplyPatchTool::new();
        let out = tool
            .execute(
                json!({
                    "file_path": "f.txt",
                    "patch": "@@ -1,2 +1,2 @@\n-hello\n+hi\n world\n"
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["mode"], "applied");
        assert_eq!(v["summary"]["files_changed"], 1);
        assert!(
            v["files"][0]["diff"]
                .as_str()
                .is_some_and(|d| d.contains("+hi"))
        );
        let disk = std::fs::read_to_string(dir.path().join("f.txt")).expect("read");
        assert_eq!(disk, "hi\nworld\n");
    }

    #[tokio::test]
    async fn rejects_multi_file_when_headers_imply_two() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = ApplyPatchTool::new();
        let p = r#"--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-x
+y
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-u
+v
"#;
        let out = tool
            .execute(
                json!({ "file_path": "a.txt", "patch": p }),
                &ctx(dir.path()),
            )
            .await;
        assert!(
            matches!(out, ToolOutput::Error { .. }),
            "expected error: {out:?}"
        );
    }

    #[tokio::test]
    async fn dry_run_does_not_modify_file() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "hello\nworld\n").expect("w");
        let tool = ApplyPatchTool::new();
        let out = tool
            .execute(
                json!({
                    "file_path": "f.txt",
                    "patch": "@@ -1,2 +1,2 @@\n-hello\n+hi\n world\n",
                    "dry_run": true
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["type"], "file_change_set");
        assert_eq!(v["mode"], "dry_run");
        let disk = std::fs::read_to_string(dir.path().join("f.txt")).expect("read");
        assert_eq!(disk, "hello\nworld\n");
    }

    #[tokio::test]
    async fn dry_run_path_escape_still_fails() {
        let dir = tempfile::tempdir().expect("tmp");
        let inner = dir.path().join("inner");
        std::fs::create_dir_all(&inner).expect("mkdir");
        std::fs::write(dir.path().join("x.txt"), "a\n").expect("w");
        let tool = ApplyPatchTool::new();
        let out = tool
            .execute(
                json!({
                    "file_path": "../x.txt",
                    "patch": "@@ -1,1 +1,1 @@\n-a\n+b\n",
                    "dry_run": true
                }),
                &ctx(&inner),
            )
            .await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::PathEscape);
    }
}
