//! Lists or reads Markdown files in `.akmon/specs/`.

use std::path::PathBuf;
use std::sync::OnceLock;

use akmon_core::Permission;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::fs;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

fn read_spec_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::ReadFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

fn specs_base_rel() -> &'static str {
    ".akmon/specs"
}

/// Lists `.akmon/specs/*.md` or returns the body of one spec by name.
pub struct ReadSpecTool;

impl ReadSpecTool {
    /// New tool instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ReadSpecTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadSpecTool {
    fn name(&self) -> &str {
        "read_spec"
    }

    fn description(&self) -> &str {
        "List Markdown specs under `.akmon/specs/` or read one by `name` (without `.md`)."
    }

    fn required_permissions(&self) -> &[Permission] {
        read_spec_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Optional base filename without `.md`. When omitted, lists all specs." }
            },
            "required": []
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let rel_dir = specs_base_rel();
        let resolved_dir = match ctx.resolve_path(rel_dir) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("sandbox: {e}"),
                };
            }
        };

        if let Some(name_raw) = args.get("name").and_then(|v| v.as_str()) {
            let t = name_raw.trim();
            if t.is_empty() {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "name must be non-empty when provided".into(),
                };
            }
            if t.contains('/') || t.contains('\\') || t.contains("..") {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "name must not contain path separators or '..'".into(),
                };
            }
            let rel_path = format!("{rel_dir}/{}.md", t.trim_end_matches(".md"));
            let full = match ctx.resolve_path(&rel_path) {
                Ok(p) => p,
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: format!("sandbox: {e}"),
                    };
                }
            };
            match fs::read_to_string(&full).await {
                Ok(body) => ToolOutput::Success {
                    content: format!("### {rel_path}\n\n{body}"),
                },
                Err(e) => ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("read {rel_path}: {e}"),
                },
            }
        } else {
            match fs::read_dir(&resolved_dir).await {
                Ok(mut entries) => {
                    let mut names = Vec::new();
                    while let Ok(Some(e)) = entries.next_entry().await {
                        let p = e.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("md")
                            && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                        {
                            names.push(stem.to_string());
                        }
                    }
                    names.sort();
                    if names.is_empty() {
                        ToolOutput::Success {
                            content: "(no specs in `.akmon/specs/`)".into(),
                        }
                    } else {
                        ToolOutput::Success {
                            content: names.join("\n"),
                        }
                    }
                }
                Err(_) => ToolOutput::Success {
                    content: "(no `.akmon/specs/` directory yet)".into(),
                },
            }
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

    #[tokio::test]
    async fn read_rejects_path_like_name() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = ReadSpecTool::new();
        for name in ["../x", "a/b", r"c\d"] {
            let out = tool
                .execute(json!({ "name": name }), &ctx(dir.path()))
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
    async fn list_and_read_specs() {
        let dir = tempfile::tempdir().expect("tmp");
        let specs = dir.path().join(".akmon/specs");
        tokio::fs::create_dir_all(&specs).await.expect("mkdir");
        tokio::fs::write(specs.join("alpha.md"), b"one")
            .await
            .expect("w");
        tokio::fs::write(specs.join("beta.md"), b"two")
            .await
            .expect("w");

        let tool = ReadSpecTool::new();
        let list = tool.execute(json!({}), &ctx(dir.path())).await;
        assert!(
            matches!(list, ToolOutput::Success { ref content } if content == "alpha\nbeta"),
            "{list:?}"
        );

        let one = tool
            .execute(json!({ "name": "alpha" }), &ctx(dir.path()))
            .await;
        assert!(
            matches!(one, ToolOutput::Success { ref content } if content.contains("one")),
            "{one:?}"
        );
    }
}
