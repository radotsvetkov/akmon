//! **Internal and unstable** — Akmon core primitives (policy, sandbox, secrets, audit).
//!
//! This crate is part of the Akmon workspace. The supported integration surface for
//! external tools in v1 is the **Akmon binary** (CLI, JSON output, exit codes, audit
//! files), not this library API.
//!
//! # Modules
//!
//! - [`secret`] — zeroizing [`Secret`] wrapper
//! - [`sandbox`] — [`Sandbox`] path boundary checks
//! - [`permission`] — [`Permission`] requests
//! - [`audit`] — [`AuditEvent`] records
//! - [`policy`] — [`PolicyEngine`] skeleton
//! - [`fsm`] — agent state machine types and transition validation
//! - [`mcp`] — MCP server configuration

#![warn(missing_docs)]

pub mod audit;
pub mod cost_estimate;
pub mod context_import;
pub mod fsm;
/// Stack detection and capped “project intelligence” snippets for model context.
pub mod lang_profile;
pub mod mcp;
pub mod permission;
pub mod policy;
pub mod project;
pub mod sandbox;
pub mod secret;

pub use audit::{
    AuditEvent, InteractivePolicyReply, PolicyVerdict, ToolOutcomeKind, write_audit_jsonl,
};
pub use cost_estimate::estimate_cost_usd;
pub use context_import::{
    CONTEXT_FILE_MAX_BYTES, ContextFile, ContextScan, ToolOrigin, primary_tool_from_files,
    scan_context_files, strip_mdc_style_frontmatter,
};
pub use fsm::{
    AgentConfig, AgentError, AgentEvent, AgentState, check_iteration_limit, validate_transition,
};
pub use lang_profile::{
    ArchitecturePattern, DataTool, Database, DatabaseAbstraction, Framework, FrameworkProfile,
    LangProfile, Language, LanguageProfile, ProjectProfile, build_project_profile,
    detect_architecture_hints, detect_data_tools, detect_databases, detect_frameworks,
    detect_language, format_language_rules, format_project_intelligence_for_root,
    format_project_profile_capped, framework_profile, lang_profile, language_incremental_profile,
    language_incremental_profile_for_root,
};
pub use mcp::McpServerConfig;
pub use permission::Permission;
pub use policy::{PolicyConfig, PolicyDecision, PolicyEngine, PolicyEngineError, PolicyEngineMode};
pub use sandbox::{Sandbox, SandboxError};
pub use secret::Secret;

pub use project::{ensure_dot_akmon_layout, save_plan_markdown, task_slug_for_plan_filename};
