//! Tool metadata for provider APIs (Ollama `tools`, OpenAI-style function calling, …).

use serde::{Deserialize, Serialize};

/// Declares a callable tool for the model: name, description, and JSON Schema for `parameters`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Stable tool identifier (e.g. `read_file`).
    pub name: String,
    /// Short description shown to the model.
    pub description: String,
    /// JSON Schema object for the tool arguments (typically `type: object` with `properties`).
    pub parameters: serde_json::Value,
}
