//! Built-in tools and the [`Tool`] trait (filesystem first; shell/network/MCP later).

#![warn(missing_docs)]

mod context;
mod list_directory;
mod output;
mod read_file;
mod search;
mod shell;
mod write_file;

pub use context::ToolContext;
pub use list_directory::ListDirectoryTool;
pub use output::{ToolErrorCode, ToolOutput};
pub use read_file::{ReadFileTool, DEFAULT_MAX_READ_BYTES};
pub use search::{SearchTool, DEFAULT_MAX_SEARCH_FILE_BYTES, DEFAULT_MAX_SEARCH_RESULTS};
pub use shell::ShellTool;
pub use write_file::WriteFileTool;

use akmon_core::Permission;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

/// One callable capability the agent may invoke (with JSON args and sandbox-aware context).
#[async_trait]
pub trait Tool: Send + Sync {
    /// Stable tool id (e.g. `read_file`) used in transcripts and policy.
    fn name(&self) -> &str;

    /// Short natural-language description shown to the model in tool listings.
    fn description(&self) -> &str;

    /// Declares which [`Permission`] variants this tool may request at runtime (paths filled in when executing).
    fn required_permissions(&self) -> &[Permission];

    /// JSON Schema object for this tool's arguments (`type`, `properties`, `required`, …).
    ///
    /// Default: empty object schema (`{}`).
    fn parameters_schema(&self) -> JsonValue {
        JsonValue::Object(serde_json::Map::new())
    }

    /// Runs the tool with parsed JSON arguments and shared [`ToolContext`].
    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput;
}
