//! Reduces the tool registry for small local models (fewer definitions → faster turns).

use std::sync::Arc;

use akmon_models::looks_like_ollama_model;
use akmon_tools::Tool;

const OLLAMA_CORE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "write_file",
    "edit",
    "shell",
    "search",
    "list_directory",
];

/// Returns a subset of `tools` appropriate for the model id.
#[must_use]
pub fn tools_for_model_id(model_id: &str, tools: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
    if !looks_like_ollama_model(model_id) {
        return tools.to_vec();
    }
    let filtered: Vec<Arc<dyn Tool>> = tools
        .iter()
        .filter(|t| OLLAMA_CORE_TOOL_NAMES.contains(&t.name()))
        .cloned()
        .collect();
    if filtered.is_empty() {
        tools.to_vec()
    } else {
        filtered
    }
}
