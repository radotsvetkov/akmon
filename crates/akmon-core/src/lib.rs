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
pub mod fsm;
pub mod mcp;
pub mod permission;
pub mod policy;
pub mod project;
pub mod sandbox;
pub mod secret;

pub use audit::{write_audit_jsonl, AuditEvent, PolicyVerdict, ToolOutcomeKind};
pub use fsm::{
    check_iteration_limit, validate_transition, AgentConfig, AgentError, AgentEvent, AgentState,
};
pub use mcp::McpServerConfig;
pub use permission::Permission;
pub use policy::{
    PolicyConfig, PolicyDecision, PolicyEngine, PolicyEngineError, PolicyEngineMode,
};
pub use sandbox::{Sandbox, SandboxError};
pub use secret::Secret;
