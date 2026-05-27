// Akmon tools crate — v1.1
//! Built-in tools and the [`Tool`] trait (filesystem, optional shell, `web_fetch`, and MCP client proxies).

#![warn(missing_docs)]

mod apply_patch;
mod ask_followup;
mod context;
mod diff_render;
mod edit;
mod file_change_set;
mod git;
pub mod journaling;
mod list_directory;
mod mcp_client;
mod memory_write;
mod output;
mod patch;
mod read_file;
mod read_spec;
mod schema_validate;
mod search;
#[cfg(feature = "semantic-index")]
mod semantic_search;
mod shell;
mod todo_write;
mod web_fetch;
mod write_file;
mod write_spec;

pub use apply_patch::ApplyPatchTool;
pub use ask_followup::AskFollowupTool;
pub use context::{ToolContext, project_hash_for_root};
pub use diff_render::{colorize_unified_diff, render_diff, unified_diff_text};
pub use edit::EditTool;
pub use git::{GitTool, try_auto_commit_after_file_tool};
pub use journaling::JournalingTool;
pub use list_directory::ListDirectoryTool;
pub use mcp_client::{McpTool, discover_mcp_tools};
pub use memory_write::{MemoryWriteTool, format_relevant_memories_block, load_relevant_memories};
pub use output::{ToolErrorCode, ToolOutput};
pub use patch::{PatchTool, patch_write_relative_paths};
pub use read_file::{DEFAULT_MAX_READ_BYTES, ReadFileTool};
pub use read_spec::ReadSpecTool;
pub use schema_validate::validate_tool_arguments;
pub use search::{DEFAULT_MAX_SEARCH_FILE_BYTES, DEFAULT_MAX_SEARCH_RESULTS, SearchTool};
#[cfg(feature = "semantic-index")]
pub use semantic_search::SemanticSearchTool;
pub use shell::ShellTool;
pub use todo_write::{
    TodoItem, TodoStatus, TodoWriteTool, format_active_tasks_block, load_current_todos,
};
pub use web_fetch::{WebFetchTool, validate_url};
pub use write_file::WriteFileTool;
pub use write_spec::{WriteSpecTool, relative_markdown_path_for_spec_name};

use std::sync::Arc;

use akmon_core::Permission;
use async_trait::async_trait;
use serde_json::Value as JsonValue;

/// MCP governance context for one tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPolicyContext {
    /// MCP server name where this tool is hosted.
    pub server: String,
    /// MCP tool name on that server.
    pub tool: String,
}

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

    /// MCP governance context when this tool is an MCP proxy.
    ///
    /// Non-MCP tools return `None`.
    fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
        None
    }

    /// Optional canonical bytes describing observable side effects this
    /// invocation produced beyond its returned [`ToolOutput`]
    /// (filesystem changes, network requests, process spawns, etc.).
    ///
    /// Default returns None. Tools that produce side effects MAY
    /// override to return canonical CBOR bytes describing those
    /// effects; the [`crate::journaling::JournalingTool`] wrapper
    /// hashes and stores these bytes, producing a `side_effects_hash`
    /// field on the resulting `ToolCall` event in the AGEF journal.
    ///
    /// Retrofitting existing tools to override this method is out of
    /// scope for v2.0. Tools that override it should ensure the
    /// returned bytes are deterministic for identical (input, output)
    /// pairs to preserve replay determinism.
    fn side_effects_manifest(
        &self,
        _input: &serde_json::Value,
        _output: &ToolOutput,
    ) -> Option<Vec<u8>> {
        None
    }
}

/// Forwards [`Tool`] so [`crate::journaling::JournalingTool`] can wrap `Arc<dyn Tool>` as its inner `T`.
#[async_trait]
impl Tool for Arc<dyn Tool> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn description(&self) -> &str {
        (**self).description()
    }

    fn required_permissions(&self) -> &[Permission] {
        (**self).required_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        (**self).parameters_schema()
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        (**self).execute(args, ctx).await
    }

    fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
        (**self).mcp_policy_context()
    }

    fn side_effects_manifest(
        &self,
        input: &serde_json::Value,
        output: &ToolOutput,
    ) -> Option<Vec<u8>> {
        (**self).side_effects_manifest(input, output)
    }
}

/// Forwards [`Tool`] so [`crate::journaling::JournalingTool`] can wrap `Box<dyn Tool>` as its inner `T`.
#[async_trait]
impl Tool for Box<dyn Tool> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn description(&self) -> &str {
        (**self).description()
    }

    fn required_permissions(&self) -> &[Permission] {
        (**self).required_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        (**self).parameters_schema()
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        (**self).execute(args, ctx).await
    }

    fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
        (**self).mcp_policy_context()
    }

    fn side_effects_manifest(
        &self,
        input: &serde_json::Value,
        output: &ToolOutput,
    ) -> Option<Vec<u8>> {
        (**self).side_effects_manifest(input, output)
    }
}

#[cfg(test)]
mod dyn_tool_forwarder_tests {
    use std::sync::{Arc, Mutex};

    use akmon_core::{Permission, PolicyEngine, PolicyEngineMode, Sandbox};
    use akmon_journal::{
        EventKind, HashAlgorithm, MemoryObjectStore, MemorySessionGraph, ObjectStore, SessionGraph,
    };
    use async_trait::async_trait;
    use serde_json::json;

    use super::{McpPolicyContext, Tool, ToolContext, ToolOutput};
    use crate::JournalingTool;

    struct ProbeTool;

    #[async_trait]
    impl Tool for ProbeTool {
        fn name(&self) -> &str {
            "probe_dyn_tool"
        }

        fn description(&self) -> &str {
            "probe"
        }

        fn required_permissions(&self) -> &[Permission] {
            &[]
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object", "probe": true})
        }

        fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
            Some(McpPolicyContext {
                server: "srv".into(),
                tool: "t".into(),
            })
        }

        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::Success {
                content: "probe-out".into(),
            }
        }

        fn side_effects_manifest(
            &self,
            _input: &serde_json::Value,
            _output: &ToolOutput,
        ) -> Option<Vec<u8>> {
            Some(vec![0xab, 0xcd])
        }
    }

    #[tokio::test]
    async fn t_arc_dyn_forwarder_routes_tool_methods() {
        let arc: Arc<dyn Tool> = Arc::new(ProbeTool);
        assert_eq!(arc.name(), "probe_dyn_tool");
        assert_eq!(arc.description(), "probe");
        assert!(arc.required_permissions().is_empty());
        assert_eq!(
            arc.parameters_schema(),
            json!({"type": "object", "probe": true})
        );
        let mcp = arc.mcp_policy_context().expect("mcp");
        assert_eq!(mcp.server, "srv");
        assert_eq!(mcp.tool, "t");
        let out = arc
            .execute(
                json!({}),
                &ToolContext::new(
                    Sandbox::new(std::env::temp_dir()),
                    Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
                ),
            )
            .await;
        assert!(matches!(out, ToolOutput::Success { .. }));
        let se = arc.side_effects_manifest(&json!({}), &out);
        assert_eq!(se.as_deref(), Some(&[0xab, 0xcd][..]));
    }

    #[tokio::test]
    async fn t_box_dyn_forwarder_routes_methods_via_journaling_tool() {
        let boxed: Box<dyn Tool> = Box::new(ProbeTool);
        assert_eq!(boxed.name(), "probe_dyn_tool");
        assert_eq!(
            boxed.parameters_schema(),
            json!({"type": "object", "probe": true})
        );
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sid = uuid::Uuid::new_v4();
        let mut mem = MemorySessionGraph::open_new(Arc::clone(&store), sid);
        let cwd_hash = store.put(b"cwd").expect("cwd");
        let config_hash = store.put(b"cfg").expect("cfg");
        mem.append(EventKind::SessionStart {
            cwd_hash,
            config_hash,
        })
        .expect("session start");
        let graph = Arc::new(Mutex::new(mem));
        let wrapped = JournalingTool::new(
            boxed,
            "probe_dyn_tool".to_owned(),
            Arc::clone(&store),
            Arc::clone(&graph),
        );
        let ctx = ToolContext::new(
            Sandbox::new(std::env::temp_dir()),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        );
        let _ = wrapped.execute(json!({"k": 1}), &ctx).await;
        let guard = graph.lock().expect("g");
        let hist = guard.history().expect("h");
        let last = hist.last().expect("event");
        match &last.1.kind {
            EventKind::ToolCall { tool_id, .. } => assert_eq!(tool_id, "probe_dyn_tool"),
            k => panic!("expected ToolCall, got {k:?}"),
        }
    }
}
