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
//! - [`policy`] — [`PolicyEngine`] enforcement
//! - [`replay`] — deterministic replay metadata hashes
//! - [`evidence`] — deterministic CI/PR run artifacts
//! - [`reliability`] — run reliability/SLO counters
//! - [`slo`] — enforceable reliability guardrails
//! - [`slo_trend`] — historical trend/regression guardrails
//! - [`fsm`] — agent state machine types and transition validation
//! - [`mcp`] — MCP server configuration

#![warn(missing_docs)]

pub mod audit;
pub mod context_import;
pub mod cost_estimate;
pub mod evidence;
pub mod fsm;
/// Stack detection and capped “project intelligence” snippets for model context.
pub mod lang_profile;
pub mod mcp;
pub mod permission;
pub mod policy;
pub mod policy_profiles;
pub mod project;
pub mod reliability;
pub mod replay;
pub mod sandbox;
pub mod secret;
pub mod slo;
pub mod slo_trend;

pub use audit::{
    AUDIT_CHAIN_SCHEMA_VERSION, AuditChainError, AuditChainRecord, AuditChainSummary, AuditEvent,
    InteractivePolicyReply, PolicyVerdict, ToolOutcomeKind, build_audit_chain, verify_audit_chain,
    verify_audit_jsonl, write_audit_jsonl,
};
pub use context_import::{
    CONTEXT_FILE_MAX_BYTES, ContextFile, ContextScan, ToolOrigin, primary_tool_from_files,
    scan_context_files, strip_mdc_style_frontmatter,
};
pub use cost_estimate::{
    ModelCostEstimateRow, TokenPricing, context_window_tokens_hint, estimate_cost_usd,
    estimate_cost_usd_from_pricing, estimate_cost_usd_with_rows, match_model_cost_row,
    pricing_for_model, resolve_token_pricing_merged,
};
pub use evidence::{
    EVIDENCE_SCHEMA_VERSION, EvidenceArtifact, EvidenceAudit, EvidencePolicy, EvidenceToolCall,
    EvidenceTools, EvidenceValidationError, EvidenceVerification, EvidenceVerificationOutcome,
    validate_evidence_artifact, validate_evidence_json, write_evidence_json,
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
pub use policy::{
    FilesystemPolicyConfig, NetworkPolicyConfig, PatternRuleSet, PolicyConfig, PolicyDecision,
    PolicyEngine, PolicyEngineError, PolicyEngineMode, ShellPolicyConfig, ToolPolicyConfig,
};
pub use policy_profiles::{
    PolicyPackError, PolicyProfileName, built_in_policy_profile, merge_policy_config,
    parse_policy_config_file,
};
pub use sandbox::{Sandbox, SandboxError};
pub use secret::Secret;

pub use project::{ensure_dot_akmon_layout, save_plan_markdown, task_slug_for_plan_filename};
pub use reliability::RunReliabilityMetrics;
pub use replay::{
    REPLAY_HASH_ALGORITHM, ReplayHashInputs, ReplayMetadata, ReplayMetadataError,
    build_replay_metadata, canonical_json_sha256, validate_replay_metadata,
    validate_replay_metadata_integrity,
};
pub use slo::{
    ReliabilitySloEvaluation, ReliabilitySloThresholds, SloCheckResult, SloCheckStatus, SloError,
    SloInputKind, SloInputMetrics, evaluate_reliability_slos, extract_slo_input_metrics,
    parse_slo_thresholds_json, parse_slo_thresholds_toml, validate_reliability_slo_thresholds,
};
pub use slo_trend::{
    BaselineMetricStats, BaselineReliabilitySummary, NormalizedReliabilityMetrics,
    RegressionGuardConfig, ReliabilityTrendEvaluation, TrendError, TrendSampleCounts,
    TrendSkippedCheck, TrendStatus, TrendViolation, aggregate_baseline_metrics,
    evaluate_reliability_trend, normalize_reliability_metrics, parse_regression_guard_config_json,
    parse_regression_guard_config_toml, validate_regression_guard_config,
};
