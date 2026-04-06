//! Apply unified diffs to existing UTF-8 files inside the sandbox (atomic writes).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use diffy::{Patch, apply};
use serde_json::{Value as JsonValue, json};
use tokio::fs;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};
use crate::write_file::atomic_write_utf8;

fn patch_file_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::WriteFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Applies standard unified diffs to one or more existing files using [`diffy`] (temp file + rename per file).
pub struct PatchTool;

impl PatchTool {
    /// Creates a new patch tool instance.
    pub fn new() -> Self {
        Self
    }
}

/// Same as [`PatchTool::new`].
impl Default for PatchTool {
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

/// Strips optional `a/` or `b/` git diff prefixes from a header path.
fn strip_ab_prefix(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix("a/")
        .or_else(|| t.strip_prefix("b/"))
        .unwrap_or(t)
}

/// True when the `---` side indicates a non-existent file (new file in git diffs).
fn is_dev_null(s: &str) -> bool {
    s == "/dev/null"
}

/// Splits a multi-file unified diff into single-file segments (diffy rejects multiple `---` in one string).
pub(crate) fn split_unified_patches(raw: &str) -> Vec<String> {
    let normalized = raw.replace('\r', "");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut starts: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("--- ") {
            starts.push(i);
        }
    }
    if starts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(lines.len());
        let chunk = lines[start..end].join("\n");
        out.push(chunk);
    }
    out
}

/// Target path string (sandbox-relative, forward slashes) from a parsed [`Patch`].
fn target_relative_path(patch: &Patch<'_, str>) -> Option<String> {
    let raw = patch.modified().or_else(|| patch.original())?;
    let stripped = strip_ab_prefix(raw);
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.replace('\\', "/"))
}

/// Lists unique sandbox-relative paths that this patch would write, for policy checks.
///
/// Returns [`None`] if the text has no `---` headers or any segment fails to parse.
pub fn patch_write_relative_paths(patch_text: &str) -> Option<Vec<PathBuf>> {
    let chunks = split_unified_patches(patch_text);
    if chunks.is_empty() {
        return None;
    }
    let mut set: HashSet<PathBuf> = HashSet::new();
    for chunk in &chunks {
        let patch = Patch::from_str(chunk).ok()?;
        let rel = target_relative_path(&patch)?;
        set.insert(PathBuf::from(rel));
    }
    let mut v: Vec<PathBuf> = set.into_iter().collect();
    v.sort();
    Some(v)
}

/// Registers the `patch` tool: unified diff application to existing files.
#[async_trait]
impl Tool for PatchTool {
    fn name(&self) -> &str {
        "patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to one or more files. Use this for larger changes that affect multiple locations in a file. The patch must be in standard unified diff format (--- a/file, +++ b/file, @@ lines)."
    }

    fn required_permissions(&self) -> &[Permission] {
        patch_file_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "A unified diff patch in standard format. File paths in the patch are relative to the project root."
                }
            },
            "required": ["patch"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let patch_text = match args.get("patch").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty \"patch\" string".into(),
                };
            }
        };

        let chunks = split_unified_patches(patch_text);
        if chunks.is_empty() {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: "patch must contain at least one unified diff file header (--- )".into(),
            };
        }

        let sandbox_root = ctx.primary_root();
        let mut file_results: Vec<JsonValue> = Vec::new();

        for (seg_ix, chunk) in chunks.iter().enumerate() {
            let patch = match Patch::from_str(chunk) {
                Ok(p) => p,
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: format!("invalid unified diff (segment {}): {e}", seg_ix + 1),
                    };
                }
            };

            if patch
                .original()
                .is_some_and(|o| is_dev_null(strip_ab_prefix(o)))
            {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "creating new files via patch is not supported; only existing files can be patched".into(),
                };
            }

            let rel_str = match target_relative_path(&patch) {
                Some(s) if !s.is_empty() => s,
                _ => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: format!(
                            "patch segment {} is missing a usable file path in ---/+++ headers",
                            seg_ix + 1
                        ),
                    };
                }
            };

            let resolved = match ctx.resolve_path(rel_str.as_str()) {
                Ok(p) => p,
                Err(SandboxError::PathEscape { .. }) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PathEscape,
                        message: format!("path escapes sandbox: {rel_str}"),
                    };
                }
                Err(SandboxError::Canonicalize(e)) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        return ToolOutput::Error {
                            code: ToolErrorCode::NotFound,
                            message: format!("path not found: {rel_str}"),
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
                        message: format!("path not found: {rel_str}"),
                    };
                }
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("could not stat path: {e}"),
                    };
                }
            };

            if !meta.is_file() {
                return ToolOutput::Error {
                    code: ToolErrorCode::NotAFile,
                    message: format!("not a regular file: {rel_str}"),
                };
            }

            let bytes = match fs::read(&resolved).await {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::NotFound,
                        message: format!("path not found: {rel_str}"),
                    };
                }
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("could not read file: {e}"),
                    };
                }
            };

            let content = match std::str::from_utf8(&bytes) {
                Ok(s) => s,
                Err(_) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::BinaryContent,
                        message: format!("file is not valid UTF-8: {rel_str}"),
                    };
                }
            };

            let new_content = match apply(content, &patch) {
                Ok(s) => s,
                Err(e) => {
                    let hunk = e.to_string();
                    return ToolOutput::Error {
                        code: ToolErrorCode::PatchFailed,
                        message: format!(
                            "could not apply patch to {rel_str}: {hunk} — file content may have changed since the patch was generated"
                        ),
                    };
                }
            };

            match atomic_write_utf8(&resolved, new_content.as_bytes()).await {
                Ok(_) => {}
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("write failed for {rel_str}: {e}"),
                    };
                }
            }

            let display_path = relative_path_display(&resolved, sandbox_root.as_path())
                .unwrap_or_else(|| rel_str.clone());

            file_results.push(json!({
                "path": display_path,
                "hunks_applied": patch.hunks().len(),
            }));
        }

        let files_patched = file_results.len();
        let payload = json!({
            "files_patched": files_patched,
            "files": file_results,
        });

        match serde_json::to_string(&payload) {
            Ok(content) => ToolOutput::Success { content },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("serialize patch result: {e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;

    use crate::read_file::ReadFileTool;

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext::new(
            Sandbox::new(root),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn apply_single_hunk() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "hello\nworld\n").expect("w");
        let patch = r#"--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
-hello
+hi
 world
"#;
        let tool = PatchTool::new();
        let out = tool
            .execute(json!({ "patch": patch }), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["files_patched"], 1);
        assert_eq!(v["files"][0]["hunks_applied"], 1);
        let disk = std::fs::read_to_string(dir.path().join("f.txt")).expect("read");
        assert_eq!(disk, "hi\nworld\n");
    }

    #[tokio::test]
    async fn apply_multi_hunk_one_file() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("t.txt"), "a\nb\nc\nd\n").expect("w");
        let patch = r#"--- a/t.txt
+++ b/t.txt
@@ -1,2 +1,3 @@
 a
+mid
 b
@@ -3,2 +4,2 @@
 c
-d
+e
"#;
        let tool = PatchTool::new();
        let out = tool
            .execute(json!({ "patch": patch }), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["files"][0]["hunks_applied"], 2);
        let disk = std::fs::read_to_string(dir.path().join("t.txt")).expect("read");
        assert_eq!(disk, "a\nmid\nb\nc\ne\n");
    }

    #[tokio::test]
    async fn wrong_context_patch_failed() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "one\ntwo\n").expect("w");
        let patch = r#"--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
-nope
+yes
 two
"#;
        let tool = PatchTool::new();
        let out = tool
            .execute(json!({ "patch": patch }), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { code, message } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::PatchFailed);
        assert!(
            message.contains("hunk") || message.contains("Hunk"),
            "message={message:?}"
        );
    }

    #[tokio::test]
    async fn path_outside_sandbox() {
        let dir = tempfile::tempdir().expect("tmp");
        let inner = dir.path().join("inner");
        std::fs::create_dir_all(&inner).expect("mkdir");
        std::fs::write(dir.path().join("x.txt"), "a\n").expect("w");
        let patch = r#"--- a/../x.txt
+++ b/../x.txt
@@ -1,1 +1,1 @@
-a
+b
"#;
        let tool = PatchTool::new();
        let out = tool.execute(json!({ "patch": patch }), &ctx(&inner)).await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::PathEscape);
    }

    #[tokio::test]
    async fn missing_file_not_found() {
        let dir = tempfile::tempdir().expect("tmp");
        let patch = r#"--- a/missing.txt
+++ b/missing.txt
@@ -0,0 +1,1 @@
+x
"#;
        let tool = PatchTool::new();
        let out = tool
            .execute(json!({ "patch": patch }), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::NotFound);
    }

    #[tokio::test]
    async fn empty_patch_invalid_args() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = PatchTool::new();
        let out = tool.execute(json!({ "patch": "" }), &ctx(dir.path())).await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::InvalidArgs);
    }

    #[tokio::test]
    async fn read_file_after_patch() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("r.txt"), "alpha\n").expect("w");
        let patch = r#"--- a/r.txt
+++ b/r.txt
@@ -1,1 +1,1 @@
-alpha
+beta
"#;
        let p = PatchTool::new();
        let r = ReadFileTool::new();
        let c = ctx(dir.path());
        let out = p.execute(json!({ "patch": patch }), &c).await;
        assert!(matches!(out, ToolOutput::Success { .. }));
        let read_out = r.execute(json!({ "path": "r.txt" }), &c).await;
        assert_eq!(
            read_out,
            ToolOutput::Success {
                content: "beta\n".into(),
            }
        );
    }

    #[tokio::test]
    async fn two_file_patch() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("a.txt"), "olda\n").expect("w");
        std::fs::write(dir.path().join("b.txt"), "oldb\n").expect("w");
        let patch = r#"--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,1 @@
-olda
+newa
--- a/b.txt
+++ b/b.txt
@@ -1,1 +1,1 @@
-oldb
+newb
"#;
        let tool = PatchTool::new();
        let out = tool
            .execute(json!({ "patch": patch }), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["files_patched"], 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt"))
                .expect("r")
                .trim_end(),
            "newa"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b.txt"))
                .expect("r")
                .trim_end(),
            "newb"
        );
    }
}
