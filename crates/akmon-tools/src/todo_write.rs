//! Persist session todo lists under `.akmon/todos/`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use akmon_core::Permission;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// One row in the session todo list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItem {
    /// Stable id (e.g. `1`, `auth`).
    pub id: String,
    /// Human-readable description.
    pub task: String,
    /// Current state in the workflow.
    pub status: TodoStatus,
}

/// Todo lifecycle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Currently being worked on.
    InProgress,
    /// Finished.
    Completed,
}

/// Create or replace the session todo list on disk.
pub struct TodoWriteTool;

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todo_write"
    }

    fn description(&self) -> &str {
        "Create or update the task list for this session. \
         Use at the start of multi-step tasks to track progress. \
         Call with the full updated list each time — this replaces the previous list."
    }

    fn required_permissions(&self) -> &[Permission] {
        static PERMS: OnceLock<[Permission; 1]> = OnceLock::new();
        PERMS
            .get_or_init(|| {
                [Permission::WriteFile {
                    path: PathBuf::from(".akmon/todos"),
                }]
            })
            .as_slice()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "task": {"type": "string"},
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["id", "task", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let todos: Vec<TodoItem> = match input.get("todos") {
            Some(v) => match serde_json::from_value(v.clone()) {
                Ok(t) => t,
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: format!("Invalid todos: {e}"),
                    };
                }
            },
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing \"todos\" array".into(),
                };
            }
        };

        let todos_dir = ctx.primary_root().join(".akmon/todos");
        if let Err(e) = std::fs::create_dir_all(&todos_dir) {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("create .akmon/todos: {e}"),
            };
        }
        let path = todos_dir.join(format!("{}.json", ctx.session_id_short()));
        let json = match serde_json::to_string_pretty(&todos) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("serialize todos: {e}"),
                };
            }
        };
        if let Err(e) = std::fs::write(&path, &json) {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("write {}: {e}", path.display()),
            };
        }

        let pending = todos
            .iter()
            .filter(|t| t.status == TodoStatus::Pending)
            .count();
        let in_progress = todos
            .iter()
            .filter(|t| t.status == TodoStatus::InProgress)
            .count();
        let completed = todos
            .iter()
            .filter(|t| t.status == TodoStatus::Completed)
            .count();

        ToolOutput::Success {
            content: format!(
                "Todos updated: {} pending, {} in progress, {} completed",
                pending, in_progress, completed
            ),
        }
    }
}

/// Loads todos for `session_id` if the JSON file exists.
#[must_use]
pub fn load_session_todos(project_root: &Path, session_id: Uuid) -> Option<Vec<TodoItem>> {
    let short: String = session_id.as_simple().to_string().chars().take(8).collect();
    let path = project_root.join(".akmon/todos").join(format!("{short}.json"));
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Formats active (non-completed) todos for system prompt injection.
#[must_use]
pub fn format_active_tasks_block(project_root: &Path, session_id: Uuid) -> Option<String> {
    let todos = load_session_todos(project_root, session_id)?;
    let active: Vec<_> = todos
        .iter()
        .filter(|t| t.status != TodoStatus::Completed)
        .collect();
    if active.is_empty() {
        return None;
    }
    let todo_text = active
        .iter()
        .map(|t| {
            let sym = match t.status {
                TodoStatus::Pending => "○",
                TodoStatus::InProgress => "⟳",
                TodoStatus::Completed => "✓",
            };
            format!("  {sym} [{}] {}", t.id, t.task)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("=== Active Tasks ===\n{todo_text}\n==="))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::{PolicyEngine, PolicyEngineMode};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn ctx(dir: &std::path::Path, sid: Uuid) -> ToolContext {
        let sandbox = akmon_core::Sandbox::with_git_root(dir.to_path_buf(), false);
        let policy = Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive));
        ToolContext::new(sandbox, policy).with_session(sid, true)
    }

    #[tokio::test]
    async fn todo_write_creates_file() {
        let dir = TempDir::new().unwrap();
        let sid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let tool = TodoWriteTool;
        let out = tool
            .execute(
                serde_json::json!({
                    "todos": [
                        {"id": "a", "task": "one", "status": "pending"}
                    ]
                }),
                &ctx(dir.path(), sid),
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }));
        let path = dir.path().join(".akmon/todos/00000000.json");
        let raw = std::fs::read_to_string(&path).expect("file");
        assert!(raw.contains("one"));
        assert!(raw.contains("pending"));
    }

    #[tokio::test]
    async fn todo_write_replaces_previous_list() {
        let dir = TempDir::new().unwrap();
        let sid = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let tool = TodoWriteTool;
        let _ = tool
            .execute(
                serde_json::json!({
                    "todos": [{"id": "1", "task": "first", "status": "completed"}]
                }),
                &ctx(dir.path(), sid),
            )
            .await;
        let _ = tool
            .execute(
                serde_json::json!({
                    "todos": [{"id": "2", "task": "second", "status": "pending"}]
                }),
                &ctx(dir.path(), sid),
            )
            .await;
        let todos = load_session_todos(dir.path(), sid).expect("loaded");
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, "2");
    }

    #[test]
    fn completed_not_in_active_block() {
        let dir = TempDir::new().unwrap();
        let sid = Uuid::nil();
        let path = dir.path().join(".akmon/todos");
        std::fs::create_dir_all(&path).unwrap();
        let short: String = sid.as_simple().to_string().chars().take(8).collect();
        let todos = vec![TodoItem {
            id: "x".into(),
            task: "done".into(),
            status: TodoStatus::Completed,
        }];
        std::fs::write(
            path.join(format!("{short}.json")),
            serde_json::to_string_pretty(&todos).unwrap(),
        )
        .unwrap();
        assert!(format_active_tasks_block(dir.path(), sid).is_none());
    }
}
