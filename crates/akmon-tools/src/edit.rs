//! Replace exactly one occurrence of a substring in a UTF-8 file (atomic write).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;

use crate::Tool;
use crate::context::ToolContext;
use crate::diff_render::unified_diff_text;
use crate::file_change_set::{ChangeSetMode, FileChange, FileChangeSet, diff_stats_from_unified};
use crate::output::{ToolErrorCode, ToolOutput};
use crate::write_file::atomic_write_utf8;

fn edit_file_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::WriteFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Replaces exactly one occurrence of `old_str` with `new_str` in a sandbox file using a temp file + rename.
pub struct EditTool;

impl EditTool {
    /// Creates a new edit tool instance.
    pub fn new() -> Self {
        Self
    }
}

/// Same as [`EditTool::new`].
impl Default for EditTool {
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

/// Counts non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn find_closest_line_hint(content: &str, search: &str) -> String {
    let first_line = search.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return "Tip: use read_file to see current content.".into();
    }
    for (i, line) in content.lines().enumerate() {
        let t = line.trim();
        if t.contains(first_line) || first_line.contains(t) {
            return format!("Closest match found at line {}:\n{}", i + 1, line);
        }
    }
    "The first line of old_str was not found. The file may have changed; use read_file first."
        .into()
}

/// Registers the `edit` tool: single exact substring replacement with an atomic file write.
#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replace an exact string in a file with new content. The old_str must match exactly once in the file — no more, no less. Use search first to find the exact string before editing. Prefer this over write_file for all changes to existing files."
    }

    fn required_permissions(&self) -> &[Permission] {
        edit_file_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the file to edit"
                },
                "old_str": {
                    "type": "string",
                    "description": "Required non-empty exact UTF-8 substring from the file (copy from read_file). Must occur exactly once. Never pass \"\" or whitespace-only — use write_file to replace a whole file."
                },
                "new_str": {
                    "type": "string",
                    "description": "The string to replace old_str with. Can be empty to delete old_str."
                },
                "dry_run": {
                    "type": "boolean",
                    "default": false,
                    "description": "Validate and generate diff payload without writing file changes."
                }
            },
            "required": ["path", "old_str", "new_str"]
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

        let old_str = match args.get("old_str").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            Some(_) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "edit: \"old_str\" must be a non-empty exact snippet from the file (read it first, copy the text to change verbatim). Empty old_str is invalid — use write_file for full rewrites.".into(),
                };
            }
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing \"old_str\" string".into(),
                };
            }
        };

        let new_str = match args.get("new_str").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing \"new_str\" string (use empty string to delete old_str)"
                        .into(),
                };
            }
        };
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

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
                    message: format!("path not found: {path_str}"),
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
                message: format!("not a regular file: {path_str}"),
            };
        }

        let bytes = match fs::read(&resolved).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return ToolOutput::Error {
                    code: ToolErrorCode::NotFound,
                    message: format!("path not found: {path_str}"),
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
                    message: format!("file is not valid UTF-8: {path_str}"),
                };
            }
        };

        let n = count_occurrences(content, old_str);
        if n == 0 {
            let preview_len = old_str.len().min(200);
            let searched = &old_str[..preview_len];
            let hint = find_closest_line_hint(content, old_str);
            return ToolOutput::Error {
                code: ToolErrorCode::NotFound,
                message: format!(
                    "The text to replace was not found in {path_str}.\n\
                     \n\
                     You searched for:\n\
                     {searched}\n\
                     \n\
                     {hint}\n\
                     \n\
                     Common causes:\n\
                     - The file was already modified by a previous edit\n\
                     - Whitespace or indentation differs from the file\n\
                     - The function or variable was renamed\n\
                     \n\
                     Use read_file to see the current content, then retry with the exact text as it appears now.",
                ),
            };
        }
        if n >= 2 {
            return ToolOutput::Error {
                code: ToolErrorCode::AmbiguousMatch,
                message: format!(
                    "old_str matches {n} times in {path_str} — add more context to make it unique"
                ),
            };
        }

        let updated = content.replacen(old_str, new_str, 1);

        let rel = match relative_path_display(&resolved, ctx.primary_root().as_ref()) {
            Some(r) => r,
            None => path_str.to_string(),
        };

        let unified = unified_diff_text(content, &updated, rel.as_str());
        let (lines_added, lines_removed, lines_changed) = diff_stats_from_unified(&unified);

        if !dry_run {
            match atomic_write_utf8(&resolved, updated.as_bytes()).await {
                Ok(_) => {}
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::PermissionDenied,
                        message: format!("write failed: {e}"),
                    };
                }
            }
        }

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
                message: format!("serialize edit result: {e}"),
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

    fn tmp_prefix_garbage(parent: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(parent)? {
            let e = e?;
            let name = e.file_name();
            let s = name.to_string_lossy();
            if s.starts_with(".akmon-w-") {
                out.push(e.path());
            }
        }
        Ok(out)
    }

    #[tokio::test]
    async fn replaces_exact_match() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "alpha\nBETA\ngamma\n").expect("w");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "BETA",
                    "new_str": "delta"
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["type"], "file_change_set");
        assert_eq!(v["mode"], "applied");
        assert_eq!(v["summary"]["files_changed"], 1);
        assert!(
            v["files"][0]["diff"]
                .as_str()
                .is_some_and(|d| d.contains("-BETA") && d.contains("+delta"))
        );
        let disk = std::fs::read_to_string(dir.path().join("f.txt")).expect("read");
        assert_eq!(disk, "alpha\ndelta\ngamma\n");
    }

    #[tokio::test]
    async fn nonexistent_file_not_found() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "nope.txt",
                    "old_str": "x",
                    "new_str": "y"
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::NotFound);
    }

    #[tokio::test]
    async fn old_str_missing_not_found_message() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "only this").expect("w");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "zzz",
                    "new_str": "q"
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Error { code, message } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::NotFound);
        assert!(
            message.contains("not found") && message.contains("searched for"),
            "message={message:?}"
        );
    }

    #[tokio::test]
    async fn duplicate_old_str_ambiguous() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "foo foo").expect("w");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "foo",
                    "new_str": "bar"
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Error { code, message } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::AmbiguousMatch);
        assert!(message.contains("2 times"), "message={message:?}");
    }

    #[tokio::test]
    async fn path_escape() {
        let dir = tempfile::tempdir().expect("tmp");
        let inner = dir.path().join("inner");
        std::fs::create_dir_all(&inner).expect("mkdir");
        std::fs::write(dir.path().join("x.txt"), "a").expect("w");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "../x.txt",
                    "old_str": "a",
                    "new_str": "b"
                }),
                &ctx(&inner),
            )
            .await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::PathEscape);
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_temp_files() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "one two").expect("w");
        let before = tmp_prefix_garbage(dir.path()).expect("list");
        assert!(before.is_empty(), "unexpected tmp before: {before:?}");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "two",
                    "new_str": "2"
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }), "{out:?}");
        let after = tmp_prefix_garbage(dir.path()).expect("list");
        assert!(
            after.is_empty(),
            "temp files should be renamed away: {after:?}"
        );
    }

    #[tokio::test]
    async fn empty_new_str_deletes_old_str() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "axb").expect("w");
        let tool = EditTool::new();
        let out = tool
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "x",
                    "new_str": ""
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }), "{out:?}");
        let disk = std::fs::read_to_string(dir.path().join("f.txt")).expect("read");
        assert_eq!(disk, "ab");
    }

    #[tokio::test]
    async fn read_file_sees_updated_content() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "keep CHANGE end").expect("w");
        let edit = EditTool::new();
        let read = ReadFileTool::new();
        let c = ctx(dir.path());
        let out = edit
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "CHANGE",
                    "new_str": "OK"
                }),
                &c,
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }));
        let read_out = read.execute(json!({ "path": "f.txt" }), &c).await;
        assert_eq!(
            read_out,
            ToolOutput::Success {
                content: "keep OK end".into(),
            }
        );
    }

    #[tokio::test]
    async fn dry_run_does_not_modify_file() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("f.txt"), "keep CHANGE end").expect("w");
        let edit = EditTool::new();
        let out = edit
            .execute(
                json!({
                    "path": "f.txt",
                    "old_str": "CHANGE",
                    "new_str": "OK",
                    "dry_run": true
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["mode"], "dry_run");
        let disk = std::fs::read_to_string(dir.path().join("f.txt")).expect("read");
        assert_eq!(disk, "keep CHANGE end");
    }
}
