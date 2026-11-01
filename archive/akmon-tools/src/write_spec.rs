//! Writes design notes into `.akmon/specs/{name}.md` (sandbox-relative).

use std::path::PathBuf;
use std::sync::OnceLock;

use akmon_core::Permission;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

fn write_spec_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::WriteFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

fn specs_base_rel() -> &'static str {
    ".akmon/specs"
}

/// Creates or overwrites one Markdown spec file under `.akmon/specs/`.
pub struct WriteSpecTool;

impl WriteSpecTool {
    /// New tool instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for WriteSpecTool {
    fn default() -> Self {
        Self::new()
    }
}

fn sanitize_spec_basename(raw: &str) -> Result<String, &'static str> {
    let t = raw.trim();
    if t.is_empty() {
        return Err("empty");
    }
    if t.contains('/') || t.contains('\\') || t.contains("..") {
        return Err("unsafe_name");
    }
    let base: String = t
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-').to_string();
    if base.is_empty() {
        return Err("sanitized_empty");
    }
    if base.len() > 120 {
        return Err("too_long");
    }
    Ok(base)
}

fn sanitize_spec_name(raw: &str) -> Result<String, ToolOutput> {
    sanitize_spec_basename(raw).map_err(|code| ToolOutput::Error {
        code: ToolErrorCode::InvalidArgs,
        message: match code {
            "empty" => "name is required (safe filename, no path separators)".into(),
            "unsafe_name" => "name must not contain path separators or '..'".into(),
            "sanitized_empty" => "name produced an empty filename after sanitizing".into(),
            "too_long" => "name too long (max 120 chars)".into(),
            _ => "invalid spec name".into(),
        },
    })
}

/// Relative path `.akmon/specs/{sanitized}.md` using the same rules as [`WriteSpecTool`].
///
/// Used for policy checks and handoff so paths match the file actually written.
#[must_use]
pub fn relative_markdown_path_for_spec_name(raw: &str) -> Option<String> {
    let base = sanitize_spec_basename(raw).ok()?;
    Some(format!("{}/{}.md", specs_base_rel(), base))
}

#[async_trait]
impl Tool for WriteSpecTool {
    fn name(&self) -> &str {
        "write_spec"
    }

    fn description(&self) -> &str {
        "Write or overwrite a Markdown spec at `.akmon/specs/{name}.md`. Use for durable requirements, plans, or checklists the main agent should see on every turn."
    }

    fn required_permissions(&self) -> &[Permission] {
        write_spec_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Base filename without `.md` (e.g. `auth-flask`)." },
                "content": { "type": "string", "description": "Full Markdown body." }
            },
            "required": ["name", "content"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing string field: name".into(),
                };
            }
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing string field: content".into(),
                };
            }
        };

        let base = match sanitize_spec_name(name) {
            Ok(b) => b,
            Err(e) => return e,
        };

        let rel_dir = specs_base_rel();
        let rel_path = format!("{rel_dir}/{base}.md");

        let parent_guess = ctx.primary_root().join(rel_dir);
        if let Err(e) = fs::create_dir_all(&parent_guess).await {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("create_dir_all: {e}"),
            };
        }

        let parent_resolved = match ctx.resolve_path(rel_dir) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("sandbox: {e}"),
                };
            }
        };

        let file_name = format!("{base}.md");
        let full = parent_resolved.join(&file_name);

        let tmp_name = format!(
            ".{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmp_path = parent_resolved.join(tmp_name);

        let mut f = match fs::File::create(&tmp_path).await {
            Ok(f) => f,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("create temp: {e}"),
                };
            }
        };
        if let Err(e) = f.write_all(content.as_bytes()).await {
            let _ = fs::remove_file(&tmp_path).await;
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("write: {e}"),
            };
        }
        if let Err(e) = f.sync_all().await {
            let _ = fs::remove_file(&tmp_path).await;
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("sync: {e}"),
            };
        }
        drop(f);

        if let Err(e) = fs::rename(&tmp_path, &full).await {
            let _ = fs::remove_file(&tmp_path).await;
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("rename: {e}"),
            };
        }

        ToolOutput::Success {
            content: format!("Wrote spec `{rel_path}` ({} bytes)", content.len()),
        }
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

    #[test]
    fn relative_path_matches_sanitized_basename() {
        assert_eq!(
            relative_markdown_path_for_spec_name("Foo Bar").as_deref(),
            Some(".akmon/specs/Foo-Bar.md")
        );
        assert_eq!(relative_markdown_path_for_spec_name("../etc/passwd"), None);
        assert_eq!(relative_markdown_path_for_spec_name(""), None);
    }

    #[tokio::test]
    async fn write_rejects_path_components_in_name() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = WriteSpecTool::new();
        for name in ["a/b", r"a\b", "..", "x/../y"] {
            let out = tool
                .execute(json!({ "name": name, "content": "z" }), &ctx(dir.path()))
                .await;
            assert!(
                matches!(
                    out,
                    ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        ..
                    }
                ),
                "name {:?} -> {out:?}",
                name
            );
        }
    }

    #[tokio::test]
    async fn write_creates_file_under_akmon_specs() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = WriteSpecTool::new();
        let out = tool
            .execute(
                json!({ "name": "my-spec", "content": "# Hello" }),
                &ctx(dir.path()),
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }), "{out:?}");
        let p = dir.path().join(".akmon/specs/my-spec.md");
        let body = tokio::fs::read_to_string(&p).await.expect("read");
        assert_eq!(body, "# Hello");
    }
}
