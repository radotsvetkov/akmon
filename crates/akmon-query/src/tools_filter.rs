//! Reduces the tool registry for small local models (fewer definitions → faster turns).

use std::sync::Arc;

use akmon_models::{ToolDefinition, looks_like_ollama_model};
use akmon_tools::Tool;

/// Tools that small local models (7b–14b parameters) can use reliably without choice overload.
///
/// Names match the Akmon tool registry (`edit`, `shell`, not `edit_file` / `bash`). Tools like
/// `todo_write` / `ask_followup` are omitted because they are not registered in this codebase.
pub const LOCAL_MODEL_CORE_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit",
    "shell",
    "search",
    "list_directory",
    "read_spec",
    "write_spec",
    "spawn_subagent",
];

/// Filters [`ToolDefinition`]s right before they are placed on the wire (API request).
#[must_use]
pub fn filter_tools_for_model(model: &str, all_tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
    if !looks_like_ollama_model(model) {
        return all_tools;
    }
    let filtered: Vec<ToolDefinition> = all_tools
        .iter()
        .filter(|t| LOCAL_MODEL_CORE_TOOLS.contains(&t.name.as_str()))
        .cloned()
        .collect();
    if filtered.is_empty() {
        all_tools
    } else {
        filtered
    }
}

/// Returns a subset of `tools` appropriate for the model id.
#[must_use]
pub fn tools_for_model_id(model_id: &str, tools: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
    if !looks_like_ollama_model(model_id) {
        return tools.to_vec();
    }
    let filtered: Vec<Arc<dyn Tool>> = tools
        .iter()
        .filter(|t| LOCAL_MODEL_CORE_TOOLS.contains(&t.name()))
        .cloned()
        .collect();
    if filtered.is_empty() {
        tools.to_vec()
    } else {
        filtered
    }
}
