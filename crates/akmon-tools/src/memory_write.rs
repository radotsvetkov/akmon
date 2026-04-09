//! Persist cross-session project memory under `~/.akmon/memory/<project_hash>/`.

use std::path::Path;

use akmon_core::Permission;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// Store durable notes about the project (user prefs, decisions, patterns).
pub struct MemoryWriteTool;

#[derive(Deserialize)]
struct Input {
    #[serde(rename = "type")]
    memory_type: String,
    title: String,
    content: String,
}

#[async_trait]
impl Tool for MemoryWriteTool {
    fn name(&self) -> &str {
        "memory_write"
    }

    fn description(&self) -> &str {
        "Store a piece of knowledge about this project that should \
         persist across sessions. Use for: architectural decisions, \
         coding patterns specific to this project, user preferences, \
         known issues and their fixes. \
         Type must be: project | user | pattern | decision"
    }

    fn required_permissions(&self) -> &[Permission] {
        &[]
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "type": {
                    "type": "string",
                    "enum": ["project", "user", "pattern", "decision"]
                },
                "title": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["type", "title", "content"]
        })
    }

    async fn execute(&self, input: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let input: Input = match serde_json::from_value(input) {
            Ok(v) => v,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("Invalid input: {e}"),
                };
            }
        };
        let allowed = ["project", "user", "pattern", "decision"];
        if !allowed.contains(&input.memory_type.as_str()) {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!(
                    "type must be one of: {}",
                    allowed.join(", ")
                ),
            };
        }

        let Some(home) = dirs::home_dir() else {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: "could not resolve home directory".into(),
            };
        };
        let memory_dir = home
            .join(".akmon/memory")
            .join(ctx.project_hash());
        if let Err(e) = std::fs::create_dir_all(&memory_dir) {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("create memory dir: {e}"),
            };
        }

        let slug: String = input
            .title
            .to_lowercase()
            .replace(' ', "_")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .take(40)
            .collect();
        let filename = format!("{}__{}.md", input.memory_type, slug);
        let path = memory_dir.join(&filename);

        let md_content = format!(
            "# {} ({})\n\n{}\n",
            input.title, input.memory_type, input.content
        );
        if let Err(e) = std::fs::write(&path, &md_content) {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("write {}: {e}", path.display()),
            };
        }

        ToolOutput::Success {
            content: format!("Memory saved: {filename}"),
        }
    }
}

/// Loads memory file bodies relevant to the current task (caps at `max_files`).
#[must_use]
pub fn load_relevant_memories(memory_dir: &Path, task_keywords: &[&str], max_files: usize) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(memory_dir) else {
        return vec![];
    };
    let mut memories = vec![];
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let always_load = name.starts_with("user__");
        let keyword_match = task_keywords
            .iter()
            .any(|kw| !kw.is_empty() && name.contains(*kw));
        if !(always_load || keyword_match) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        memories.push(content);
        if memories.len() >= max_files {
            break;
        }
    }
    memories
}

/// Extracts crude keywords from the user task (whitespace tokens, len 3+).
fn task_keyword_tokens(task: &str) -> Vec<String> {
    task.split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|s| s.len() >= 3)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Builds an optional system block with memories for the prompt.
#[must_use]
pub fn format_relevant_memories_block(project_root: &Path, task: &str) -> Option<String> {
    let hash = crate::context::project_hash_for_root(project_root);
    let home = dirs::home_dir()?;
    let memory_dir = home.join(".akmon/memory").join(&hash);
    let tokens: Vec<String> = task_keyword_tokens(task);
    let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let mems = load_relevant_memories(&memory_dir, &refs, 5);
    if mems.is_empty() {
        return None;
    }
    let body = mems.join("\n\n---\n\n");
    Some(format!("=== Project memory (relevant) ===\n{body}\n==="))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn memory_write_filename_format() {
        let slug: String = "My Project Decisions"
            .to_lowercase()
            .replace(' ', "_")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .take(40)
            .collect();
        assert_eq!(
            format!("decision__{slug}.md"),
            "decision__my_project_decisions.md"
        );
    }

    #[test]
    fn memory_load_always_includes_user_type() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("user__prefs.md"),
            "# prefs\n\nbody",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("project__other.md"),
            "# other",
        )
        .unwrap();
        let m = load_relevant_memories(dir.path(), &["zzznope"], 5);
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("prefs"));
    }
}
