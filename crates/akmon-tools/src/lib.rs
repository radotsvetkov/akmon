// Akmon tools crate — v1.1
//! Built-in tools and the [`Tool`] trait (filesystem, optional shell, `web_fetch`, and MCP client proxies).

#![warn(missing_docs)]

mod context;
mod edit;
mod git;
mod list_directory;
mod mcp_client;
mod output;
mod patch;
mod read_file;
mod search;
#[cfg(feature = "semantic-index")]
mod semantic_search;
mod shell;
mod web_fetch;
mod write_file;

pub use context::ToolContext;
pub use edit::EditTool;
pub use git::{try_auto_commit_after_file_tool, GitTool};
pub use list_directory::ListDirectoryTool;
pub use mcp_client::{discover_mcp_tools, McpTool};
pub use output::{ToolErrorCode, ToolOutput};
pub use patch::{patch_write_relative_paths, PatchTool};
pub use read_file::{ReadFileTool, DEFAULT_MAX_READ_BYTES};
pub use search::{SearchTool, DEFAULT_MAX_SEARCH_FILE_BYTES, DEFAULT_MAX_SEARCH_RESULTS};
#[cfg(feature = "semantic-index")]
pub use semantic_search::SemanticSearchTool;
pub use shell::ShellTool;
pub use web_fetch::{validate_url, WebFetchTool};
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
