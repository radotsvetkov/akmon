//! Agent session: owns FSM state, provider, tools, and the main query loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use akmon_core::{
    AgentConfig, AgentError, AgentEvent, AgentState, AuditEvent, InteractivePolicyReply,
    Permission, PolicyEngineError, PolicyEngineMode, PolicyVerdict, ReplayHashInputs,
    ReplayMetadata, RunReliabilityMetrics, Sandbox, ToolOutcomeKind, build_replay_metadata,
    check_iteration_limit, estimate_cost_usd_with_rows, validate_transition,
};
use akmon_journal::{ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph};
use akmon_models::{
    CompletionConfig, CompletionStream, JournalingProvider, LlmProvider, Message, MessageRole,
    ModelError, ModelToolCall, StopReason, StreamEvent, ToolDefinition, UsageReport,
    anthropic_system_block_text, approximate_tokens, canonical_provider_id,
    looks_like_ollama_model, max_tokens_for_model, ollama_first_token_deadline_ms,
};
use akmon_tools::{
    JournalingTool, McpPolicyContext, Tool, ToolContext, ToolOutput, unified_diff_text,
};
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::context::{
    build_followup_messages, build_messages, build_subagent_followup_messages,
    build_subagent_task_messages, context_limit_for_model,
};
use crate::context_manager::ContextManager;
use crate::journal::{JournalHandle, emit_session_start, emit_user_turn};
use crate::microcompact::{
    MICROCOMPACT_KEEP_RECENT_DEFAULT, MICROCOMPACT_KEEP_RECENT_GROQ, apply_microcompact_context,
};
use crate::specs_and_handoff;
use crate::tools_filter::{filter_tools_for_model, tools_for_model_id};

/// One finished tool invocation recorded for machine-readable run summaries (CLI `--output json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolCallSummary {
    /// Registered tool name (e.g. `read_file`).
    pub name: String,
    /// Whether the tool reported success.
    pub success: bool,
    /// Short, user-facing result or error message.
    pub message: String,
}

/// Policy decision summary extracted from run audit events for evidence artifacts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicyDecisionSummary {
    /// Number of allow verdicts.
    pub allow: u64,
    /// Number of deny verdicts.
    pub deny: u64,
    /// Number of interactive/prompted policy decisions (best-effort).
    pub prompted: u64,
    /// Sanitized sample lines (permission kind + verdict + reason).
    pub decision_samples: Vec<String>,
}

/// Deterministic evidence input snapshot exported by the session layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionEvidenceData {
    /// Stable session identifier.
    pub session_id: String,
    /// Replay metadata produced for this run.
    pub replay_metadata: Option<ReplayMetadata>,
    /// Policy decision summary from audit events.
    pub policy: PolicyDecisionSummary,
    /// Chronological tool summaries from this run.
    pub tools: Vec<ToolCallSummary>,
    /// Sandbox-relative modified paths (sorted unique).
    pub files_touched: Vec<String>,
    /// Reliability/SLO counters for this run.
    pub reliability_metrics: RunReliabilityMetrics,
}

/// Why [`AgentSession::run`] stopped successfully (when [`Result`] is [`Ok`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRunExit {
    /// Normal completion (including iteration exhaustion surfaced as [`Err`] elsewhere).
    Completed,
    /// Headless `--max-budget-usd` cap tripped after the last completed model round.
    BudgetLimit,
}

/// One model tool call after policy approval, ready for [`execute_single_tool_call`].
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    /// Tool call id from the model.
    pub id: String,
    /// Registered tool name.
    pub name: String,
    /// JSON arguments object.
    pub arguments: Value,
}

/// Structured outcome from [`execute_single_tool_call`] (and policy-denied synthetic rows).
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    /// Same id as the model tool call.
    pub call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Raw tool output for the transcript.
    pub output: ToolOutput,
    /// Whether the tool reported success.
    pub success: bool,
    /// Arguments the model supplied (for post-hooks such as auto-commit).
    pub arguments: Value,
    /// Wall-clock duration of this tool call in milliseconds.
    pub latency_ms: u64,
}

#[derive(Debug, Clone)]
struct McpAuditContext {
    server: String,
    tool: String,
}

/// Builds [`CompletionConfig`] with `tools` populated and model-appropriate [`CompletionConfig::max_tokens`].
fn completion_config_for_tools(
    tools: &[Arc<dyn Tool>],
    provider: &dyn LlmProvider,
    session_id: uuid::Uuid,
    max_completion_tokens: Option<u32>,
    fallback_model: Option<String>,
) -> CompletionConfig {
    let defs: Vec<ToolDefinition> = tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.as_ref().name().to_string(),
            description: t.as_ref().description().to_string(),
            parameters: t.as_ref().parameters_schema(),
        })
        .collect();
    let defs = filter_tools_for_model(provider.completion_model_id(), defs);
    let mut cfg = CompletionConfig {
        tools: defs,
        max_tokens: max_completion_tokens
            .unwrap_or_else(|| max_tokens_for_model(provider.completion_model_id())),
        session_id: Some(session_id.to_string()),
        fallback_model,
        ..CompletionConfig::default()
    };
    if provider.name() == "ollama" {
        cfg.first_token_deadline_ms =
            ollama_first_token_deadline_ms(provider.completion_model_id());
    }
    // Apply first_token_deadline_ms from ~/.akmon/config.toml if set.
    // This overrides both the 300 s default and the Ollama model-derived value,
    // letting operators tune the deadline for slow local inference servers
    // (e.g. lemonade on AMD Strix Halo prefilling large codebases).
    if let Ok((_, global_cfg)) = akmon_config::load_user_config() {
        if let Some(deadline_ms) = global_cfg.first_token_deadline_ms {
            cfg.first_token_deadline_ms = deadline_ms;
        }
    }
    cfg
}

/// Keeps all leading system messages and only the last `context_limit_for_model` non-system rows.
fn trim_messages_for_model(model_id: &str, messages: Vec<Message>) -> Vec<Message> {
    if looks_like_ollama_model(model_id) {
        let mut out: Vec<Message> = Vec::new();
        if let Some(first_system) = messages.iter().find(|m| m.role == MessageRole::System) {
            out.push(first_system.clone());
        }
        let non_system: Vec<Message> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .cloned()
            .collect();
        let keep = 6usize;
        let start = non_system.len().saturating_sub(keep);
        out.extend(non_system[start..].iter().cloned());
        return out;
    }
    let limit = context_limit_for_model(model_id);
    if limit == usize::MAX {
        return messages;
    }
    let n_system = messages
        .iter()
        .take_while(|m| m.role == MessageRole::System)
        .count();
    if n_system >= messages.len() {
        return messages;
    }
    let prefix: Vec<Message> = messages[..n_system].to_vec();
    let rest = &messages[n_system..];
    if rest.len() <= limit {
        return messages;
    }
    let mut out = prefix;
    out.extend_from_slice(&rest[rest.len() - limit..]);
    out
}

/// Owns one running agent: configuration, FSM state, policy, model backend, tool registry, chat
/// history, audit trail, optional `AKMON.md` text, and the filesystem [`Sandbox`].
pub struct AgentSession<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    config: AgentConfig,
    state: AgentState,
    policy: Arc<akmon_core::PolicyEngine>,
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Arc<dyn Tool>>,
    sandbox: Arc<Sandbox>,
    context: Vec<Message>,
    audit_log: Vec<AuditEvent>,
    akmon_md: Option<String>,
    /// Decremented on each [`AgentEvent::ToolCallCompleted`] while in [`AgentState::ToolExecution`]; drives return to Thinking.
    parallel_tool_batch_remaining: u32,
    /// Concatenation of all assistant [`StreamEvent::TextDelta`] chunks for this run.
    result_text: String,
    /// Completed tool calls in chronological order (for JSON run reports).
    tool_call_summaries: Vec<ToolCallSummary>,
    /// Token-budget and splitting rules for automatic context compaction.
    context_manager: ContextManager,
    /// FSM state to restore after a successful [`AgentEvent::ContextSummarized`].
    post_summary_resume: Option<AgentState>,
    /// Most recent per-completion usage from the model provider (e.g. Anthropic), if any.
    last_usage: Option<UsageReport>,
    /// Sum of `input_tokens` from each [`StreamEvent::UsageReport`] in this run.
    total_input_tokens: u32,
    /// Sum of `cache_read_tokens` from each [`StreamEvent::UsageReport`] in this run.
    total_cache_read_tokens: u32,
    /// Sum of `output_tokens` from each [`StreamEvent::UsageReport`] in this run.
    total_output_tokens: u32,
    /// Heuristic cumulative USD for this [`Self::run`] (see [`akmon_core::estimate_cost_usd`]).
    total_cost_usd: f64,
    /// When set, the next model iteration must not start (headless budget cap).
    budget_stop_before_next_iteration: bool,
    /// Outcome of the last finished [`Self::run`].
    last_run_exit: SessionRunExit,
    /// When `true`, project system prompts are read-only (plan mode); tools should match.
    plan_mode: bool,
    /// Permissions the user allowed with “remember for session”; checked with equality against
    /// each new request (same path, same shell command string, etc.) before prompting again.
    permission_session_allowlist: Vec<Permission>,
    /// User chose “allow all writes” in the permission dialog for this session.
    permission_allow_all_writes: bool,
    /// Shell command prefixes approved for the whole session (from “allow prefix” in the dialog).
    permission_shell_prefixes: Vec<String>,
    /// How many automatic max-token continuations have run this turn (reset on EndTurn, ToolUse, new user turn).
    pub continuation_count: u32,
    /// Per-tool invocation counts for the current user task (reset in [`AgentSession::prepare_for_new_user_turn`]).
    pub tool_call_counts: HashMap<String, u32>,
    /// Successful [`Self::run`] completions since this session was constructed.
    pub user_turns_finished: u32,
    /// Sandbox-relative paths touched after successful file-changing tools (for handoff).
    pub modified_paths: Vec<PathBuf>,
    /// Truncated assistant text from the last finished run (`HANDOFF.md`).
    pub last_assistant_snippet: Option<String>,
    /// Deterministic replay metadata for the most recent run snapshot.
    replay_metadata: Option<ReplayMetadata>,
    /// Optional prompt-assembly fingerprint captured from the first model request shape of a run.
    prompt_assembly_fingerprint: Option<Value>,
    /// Per-run reliability metrics.
    reliability_metrics: RunReliabilityMetrics,
    /// Latency samples used to compute p95 cheaply at run scope.
    tool_latency_samples_ms: Vec<u64>,
    /// MCP context keyed by model tool-call id, for audit enrichment.
    mcp_call_context_by_id: HashMap<String, McpAuditContext>,
    /// AGEF journal store and session graph for this [`AgentSession`].
    journal: JournalHandle<S, G>,
    /// Set after [`crate::journal::emit_session_start`] succeeds in [`Self::new`]. [`Drop`] skips [`SessionEnd`](akmon_journal::EventKind::SessionEnd) until this is true.
    journal_started: AtomicBool,
    /// When `true`, [`SessionEnd`](akmon_journal::EventKind::SessionEnd) was emitted (explicit [`Self::end`] or [`Drop`]).
    ended: AtomicBool,
}

/// Agent session using the default on-disk journal from [`crate::open_default_journal_handle`].
pub type DefaultAgentSession = AgentSession<RedbObjectStore, RedbSessionGraph>;

impl<S, G> AgentSession<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    /// Creates a session in [`AgentState::Idle`] with the given dependencies, emits [`SessionStart`](akmon_journal::EventKind::SessionStart), wraps `provider` in [`JournalingProvider`](akmon_models::JournalingProvider), then wraps each tool in [`JournalingTool`](akmon_tools::JournalingTool) so every dispatch emits [`ToolCall`](akmon_journal::EventKind::ToolCall) evidence.
    #[allow(clippy::too_many_arguments)] // journal handle is an explicit construction dependency (Item 3.1b).
    pub fn new(
        config: AgentConfig,
        policy: Arc<akmon_core::PolicyEngine>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        sandbox: Arc<Sandbox>,
        akmon_md: Option<String>,
        plan_mode: bool,
        journal: JournalHandle<S, G>,
    ) -> Result<Self, AgentError> {
        let max_tokens = provider.context_window_tokens().clamp(1, 100_000);
        let fixed_system_messages = if akmon_md.is_some() { 2 } else { 1 };
        let context_manager = ContextManager {
            max_tokens,
            threshold: ContextManager::default().threshold,
            keep_recent: ContextManager::default().keep_recent,
            fixed_system_messages,
        };
        emit_session_start(&journal, &config)?;
        let provider_id = canonical_provider_id(provider.as_ref());
        let inner: Arc<dyn LlmProvider + Send + Sync> = provider;
        let wrapped: Arc<dyn LlmProvider> = Arc::new(JournalingProvider::new(
            inner,
            provider_id,
            journal.store.clone(),
            journal.graph.clone(),
        ));
        let tools = Self::journal_wrap_tools(tools, &journal);
        Ok(Self {
            config,
            state: AgentState::Idle,
            policy,
            provider: wrapped,
            tools,
            sandbox,
            context: Vec::new(),
            audit_log: Vec::new(),
            akmon_md,
            parallel_tool_batch_remaining: 0,
            result_text: String::new(),
            tool_call_summaries: Vec::new(),
            context_manager,
            post_summary_resume: None,
            last_usage: None,
            total_input_tokens: 0,
            total_cache_read_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
            budget_stop_before_next_iteration: false,
            last_run_exit: SessionRunExit::Completed,
            plan_mode,
            permission_session_allowlist: Vec::new(),
            permission_allow_all_writes: false,
            permission_shell_prefixes: Vec::new(),
            continuation_count: 0,
            tool_call_counts: HashMap::new(),
            user_turns_finished: 0,
            modified_paths: Vec::new(),
            last_assistant_snippet: None,
            replay_metadata: None,
            prompt_assembly_fingerprint: None,
            reliability_metrics: RunReliabilityMetrics::default(),
            tool_latency_samples_ms: Vec::new(),
            mcp_call_context_by_id: HashMap::new(),
            journal,
            journal_started: AtomicBool::new(true),
            ended: AtomicBool::new(false),
        })
    }

    fn journal_wrap_tools(
        tools: Vec<Box<dyn Tool>>,
        journal: &JournalHandle<S, G>,
    ) -> Vec<Arc<dyn Tool>> {
        tools
            .into_iter()
            .map(|boxed| {
                let inner: Arc<dyn Tool> = Arc::from(boxed);
                let tool_id = inner.name().to_string();
                Arc::new(JournalingTool::new(
                    inner,
                    tool_id,
                    journal.store.clone(),
                    journal.graph.clone(),
                )) as Arc<dyn Tool>
            })
            .collect()
    }

    /// Best-effort [`akmon_journal::EventKind::PermissionGate`] emission; failures are logged and ignored.
    fn warn_emit_permission_gate(
        &self,
        policy_id: &str,
        decision: &str,
        tool_name: &str,
        tool_input: &Value,
        decision_path: &str,
    ) {
        let bytes = match crate::journal::permission_gate_context_cbor(
            tool_name,
            tool_input,
            decision_path,
        ) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    target: "akmon::session",
                    error = %e,
                    "PermissionGate context serialization failed; skipping journal event"
                );
                return;
            }
        };
        if let Err(e) =
            crate::journal::emit_permission_gate(&self.journal, policy_id, decision, &bytes)
        {
            tracing::warn!(
                target: "akmon::session",
                error = %e,
                "PermissionGate journal emission failed; continuing session"
            );
        }
    }

    /// Best-effort [`akmon_journal::EventKind::AssistantTurn`] emission; failures are logged and ignored.
    fn warn_emit_assistant_turn_cbor(&self, message_text: &str, tool_calls_cbor: Option<&[u8]>) {
        if let Err(e) = crate::journal::emit_assistant_turn(
            &self.journal,
            message_text.as_bytes(),
            tool_calls_cbor,
        ) {
            tracing::warn!(
                target: "akmon::session",
                error = %e,
                "AssistantTurn journal emission failed; continuing session"
            );
        }
    }

    fn try_emit_session_end_once(
        &self,
        summary_hash: Option<akmon_journal::Hash>,
    ) -> Result<(), AgentError> {
        if self
            .ended
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }
        if let Err(e) = crate::journal::append_session_end(&self.journal, summary_hash) {
            self.ended.store(false, Ordering::SeqCst);
            return Err(e);
        }
        Ok(())
    }

    /// Emits [`SessionEnd`](akmon_journal::EventKind::SessionEnd) once; preferred over relying on [`Drop`] when a `summary_hash` exists.
    pub fn end(&self, summary_hash: Option<akmon_journal::Hash>) -> Result<(), AgentError> {
        self.try_emit_session_end_once(summary_hash)
    }

    #[cfg(test)]
    pub(crate) fn journal_history_snapshot(
        &self,
    ) -> Result<Vec<(akmon_journal::Hash, akmon_journal::Event)>, akmon_journal::JournalError> {
        let guard = self
            .journal
            .graph
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.history()
    }

    /// Clears chat context and per-turn counters; optionally deletes `.akmon/specs/*.md`.
    ///
    /// Does not reload `AKMON.md` or change the tool registry.
    pub fn clear_transcript_soft(
        &mut self,
        project_root: &Path,
        delete_specs: bool,
    ) -> std::io::Result<()> {
        self.context.clear();
        self.result_text.clear();
        self.tool_call_summaries.clear();
        self.continuation_count = 0;
        self.tool_call_counts.clear();
        self.modified_paths.clear();
        self.last_assistant_snippet = None;
        self.user_turns_finished = 0;
        if delete_specs {
            specs_and_handoff::clear_specs_dir(project_root)?;
        }
        Ok(())
    }

    fn record_run_finished_success(&mut self) {
        self.user_turns_finished = self.user_turns_finished.saturating_add(1);
        let s = self.result_text();
        self.last_assistant_snippet = if s.is_empty() {
            None
        } else if s.len() > 2000 {
            Some(s[..2000].to_string())
        } else {
            Some(s.to_string())
        };
    }

    fn write_rate_limit_handoff_note(&self, task: &str) {
        let handoff = specs_and_handoff::handoff_path(self.sandbox.primary_root());
        if let Some(parent) = handoff.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let model = self.provider.completion_model_id();
        let body = format!(
            "**Model:** {model}\n\n\
**Status:** Rate limit exhausted after 5 retries.\n\n\
**Resume:** Wait a few minutes then continue with: `akmon -c`\n\n\
**Last task:**\n\n{task}\n"
        );
        let _ = std::fs::write(handoff, body);
    }

    async fn handle_model_error_for_run(
        &mut self,
        event_tx: &mpsc::Sender<AgentEvent>,
        task: &str,
        err: ModelError,
    ) -> Result<Option<AgentError>, AgentError> {
        self.record_timeout_if_model_error(&err);
        if matches!(err, ModelError::RateLimited { .. }) {
            let resume = "Rate limit exhausted after 5 retries.\nWait a few minutes then continue with: akmon -c";
            self.apply_event(
                event_tx,
                AgentEvent::StatusInfo {
                    message: resume.into(),
                },
                task,
            )
            .await?;
            self.write_rate_limit_handoff_note(task);
            return Ok(Some(AgentError::ModelError {
                message: resume.into(),
            }));
        }

        let context_hint = match &err {
            ModelError::ContextWindowExceeded => Some(
                "Context too large for local model. Options:\n 1. Type /clear to reset and continue with less context\n 2. Switch to cloud: /model claude-haiku-4-5-20251001\n 3. Use a larger local model: /model qwen3.5:27b",
            ),
            ModelError::StreamInterrupted { message }
                if message.contains("The context may be too large for this model.") =>
            {
                Some(
                    "Context too large for local model. Options:\n 1. Type /clear to reset and continue with less context\n 2. Switch to cloud: /model claude-haiku-4-5-20251001\n 3. Use a larger local model: /model qwen3.5:27b",
                )
            }
            _ => None,
        };
        if let Some(message) = context_hint {
            self.apply_event(
                event_tx,
                AgentEvent::StatusInfo {
                    message: message.into(),
                },
                task,
            )
            .await?;
        }

        let ae = map_model_error(err);
        self.apply_event(
            event_tx,
            AgentEvent::Error {
                error: ae.clone(),
                recoverable: true,
            },
            task,
        )
        .await?;
        Ok(Some(ae))
    }

    fn note_successful_file_tool_for_handoff(&mut self, r: &ToolCallResult) {
        if !r.success {
            return;
        }
        let rel: Option<PathBuf> = match r.tool_name.as_str() {
            "write_file" | "edit" => r
                .arguments
                .get("path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from),
            "write_spec" => r
                .arguments
                .get("name")
                .and_then(|v| v.as_str())
                .and_then(akmon_tools::relative_markdown_path_for_spec_name)
                .map(PathBuf::from),
            "apply_patch" => r
                .arguments
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from),
            _ => None,
        };
        if let Some(pb) = rel
            && !self.modified_paths.contains(&pb)
        {
            self.modified_paths.push(pb);
        }
    }

    fn compose_model_messages(
        &self,
        task: &str,
        user_line_committed: bool,
        project_root: &str,
        tool_names: &[&str],
        model_id: &str,
    ) -> Vec<Message> {
        let root_path = self.sandbox.primary_root();
        let specs_owned = specs_and_handoff::load_specs_block_for_prompt(root_path);
        let handoff_owned = if self.config.subagent_style {
            None
        } else {
            specs_and_handoff::load_handoff_block_for_prompt(root_path)
        };
        let specs_ref = specs_owned.as_deref();
        let handoff_ref = handoff_owned.as_deref();
        let extras: Vec<String> = {
            let mut v = Vec::new();
            if let Some(s) = akmon_tools::format_active_tasks_block(root_path) {
                v.push(s);
            }
            if let Some(s) = akmon_tools::format_relevant_memories_block(root_path, task) {
                v.push(s);
            }
            v
        };
        if self.config.subagent_style {
            if user_line_committed {
                build_subagent_followup_messages(
                    self.akmon_md.as_deref(),
                    &self.context,
                    project_root,
                    tool_names,
                    specs_ref,
                    model_id,
                )
            } else {
                build_subagent_task_messages(
                    self.akmon_md.as_deref(),
                    task,
                    project_root,
                    tool_names,
                    specs_ref,
                    model_id,
                )
            }
        } else if user_line_committed {
            build_followup_messages(
                self.akmon_md.as_deref(),
                &self.context,
                project_root,
                tool_names,
                self.plan_mode,
                model_id,
                specs_ref,
                handoff_ref,
                &extras,
            )
        } else {
            build_messages(
                self.akmon_md.as_deref(),
                &self.context,
                task,
                project_root,
                tool_names,
                self.plan_mode,
                model_id,
                specs_ref,
                handoff_ref,
                &extras,
            )
        }
    }

    /// Returns whether this session uses read-only plan-mode system prompts.
    pub fn plan_mode(&self) -> bool {
        self.plan_mode
    }

    /// Swaps the tool registry (e.g. between plan-only and full implementation turns).
    ///
    /// Each incoming tool is wrapped in [`JournalingTool`](akmon_tools::JournalingTool) like at construction.
    pub fn replace_tools(&mut self, tools: Vec<Box<dyn Tool>>) {
        self.tools = Self::journal_wrap_tools(tools, &self.journal);
    }

    #[cfg(test)]
    pub(crate) fn test_set_permission_allow_all_writes(&mut self, allow: bool) {
        self.permission_allow_all_writes = allow;
    }

    /// Enables or disables plan-mode system prompts for subsequent model calls.
    pub fn set_plan_mode(&mut self, plan_mode: bool) {
        self.plan_mode = plan_mode;
    }

    /// Returns the stable session id from [`AgentConfig`].
    pub fn session_id(&self) -> Uuid {
        self.config.session_id
    }

    /// Full assistant text accumulated from streaming deltas across all model turns in this run.
    pub fn result_text(&self) -> &str {
        &self.result_text
    }

    /// Append-only audit events for this session (policy rows, agent steps, etc.).
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.audit_log
    }

    /// Tool completions recorded in order (includes failures and policy-denied tools).
    pub fn tool_call_summaries(&self) -> &[ToolCallSummary] {
        &self.tool_call_summaries
    }

    /// Returns the current FSM state (for tests and harnesses).
    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Shared sandbox handle (project root, path resolution).
    pub fn sandbox_arc(&self) -> Arc<Sandbox> {
        Arc::clone(&self.sandbox)
    }

    /// Model provider used for completions in this session.
    pub fn provider_arc(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.provider)
    }

    /// Copy of loaded `AKMON.md` text, if any.
    pub fn akmon_md_cloned(&self) -> Option<String> {
        self.akmon_md.clone()
    }

    /// Last [`UsageReport`] from the most recent model completion in this run, if the provider emitted one.
    pub fn last_usage(&self) -> Option<&UsageReport> {
        self.last_usage.as_ref()
    }

    /// Total billed input tokens accumulated from all [`StreamEvent::UsageReport`] events this run.
    pub fn total_input_tokens(&self) -> u32 {
        self.total_input_tokens
    }

    /// Total prompt-cache read tokens accumulated this run (non-zero when Anthropic cache hits occurred).
    pub fn total_cache_read_tokens(&self) -> u32 {
        self.total_cache_read_tokens
    }

    /// Total output tokens accumulated from all [`StreamEvent::UsageReport`] events this run.
    pub fn total_output_tokens(&self) -> u32 {
        self.total_output_tokens
    }

    /// Heuristic cumulative USD spend this [`Self::run`] (zero for unknown pricing or local/Ollama).
    pub fn total_cost_usd(&self) -> f64 {
        self.total_cost_usd
    }

    /// Success reason for the last finished [`Self::run`] (see [`SessionRunExit`]).
    pub fn last_run_exit(&self) -> SessionRunExit {
        self.last_run_exit
    }

    /// Replay metadata snapshot for the latest run.
    pub fn replay_metadata(&self) -> Option<&ReplayMetadata> {
        self.replay_metadata.as_ref()
    }

    /// Reliability counters for the latest run.
    pub fn reliability_metrics(&self) -> RunReliabilityMetrics {
        self.reliability_metrics_snapshot()
    }

    /// Session evidence snapshot suitable for artifact construction.
    pub fn evidence_data(&self) -> SessionEvidenceData {
        let mut files_touched = self
            .modified_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        files_touched.sort();
        files_touched.dedup();
        SessionEvidenceData {
            session_id: self.session_id().to_string(),
            replay_metadata: self.replay_metadata.clone(),
            policy: summarize_policy_decisions(&self.audit_log),
            tools: self.tool_call_summaries.clone(),
            files_touched,
            reliability_metrics: self.reliability_metrics_snapshot(),
        }
    }

    /// Replaces model-visible transcript rows (e.g. after loading a saved session file).
    pub fn restore_context_from_messages(&mut self, messages: Vec<Message>) {
        self.context = messages;
    }

    /// Current model-visible transcript (for persisting `~/.akmon/sessions/*.json`).
    pub fn context_messages(&self) -> &[Message] {
        &self.context
    }

    /// Allows a follow-up [`run`] after [`AgentState::Complete`] or [`AgentState::Failed`] by
    /// returning to [`AgentState::Idle`] and clearing per-turn text/tool summaries.
    ///
    /// Returns an error when the session is busy (planning, thinking, tools, etc.).
    pub fn prepare_for_new_user_turn(&mut self) -> Result<(), AgentError> {
        match self.state {
            AgentState::Complete | AgentState::Failed { .. } => {
                self.state = AgentState::Idle;
                self.parallel_tool_batch_remaining = 0;
            }
            AgentState::Idle => {}
            _ => {
                return Err(AgentError::SessionFailed {
                    message: format!("cannot start a new turn from state {:?}", self.state),
                });
            }
        }
        self.result_text.clear();
        self.tool_call_summaries.clear();
        self.continuation_count = 0;
        self.tool_call_counts.clear();
        self.total_cost_usd = 0.0;
        self.budget_stop_before_next_iteration = false;
        self.last_run_exit = SessionRunExit::Completed;
        self.prompt_assembly_fingerprint = None;
        self.reliability_metrics = RunReliabilityMetrics::default();
        self.tool_latency_samples_ms.clear();
        self.refresh_replay_metadata();
        Ok(())
    }

    /// Runs one user task: planning, one or more model completions, real tool dispatch, and
    /// streaming events until [`StopReason::EndTurn`] or an error.
    ///
    /// Requires [`AgentState::Idle`]. Emits on `event_tx` and records [`AuditEvent::AgentStep`] plus
    /// policy evaluation rows from the engine.
    ///
    /// After [`StopReason::ToolUse`], tool calls are executed (or recorded as failed), results are
    /// appended as [`MessageRole::Tool`] messages, the iteration counter increases, and
    /// [`check_iteration_limit`] runs before the next provider call.
    ///
    /// When the policy is [`PolicyEngineMode::Interactive`], [`PolicyEngineMode::AutoApproveReads`],
    /// or [`PolicyEngineMode::AutoApproveReadsAndFetch`],
    /// put the session's [`mpsc::Receiver<InteractivePolicyReply>`] in `interactive_policy_rx` as `Some` so
    /// write confirmations can be answered; the UI must send one reply after each
    /// [`AgentEvent::ConfirmationRequired`]. Use `&mut None` only when no interactive confirmations are
    /// possible (reads-only sessions may still use `Some` harmlessly).
    ///
    /// When `interrupt_after_current_tools` is [`Some`] and the flag is set after a tool batch
    /// finishes, the session emits [`AgentEvent::Done`] and returns [`Ok(())`] without another model call.
    ///
    /// After [`Self::prepare_for_new_user_turn`] succeeds and state is [`AgentState::Idle`], appends
    /// [`UserTurn`](akmon_journal::EventKind::UserTurn) to the journal (`task` as UTF-8); failure returns
    /// [`AgentError::SessionFailed`] and does not run the model loop.
    pub async fn run(
        &mut self,
        task: String,
        event_tx: mpsc::Sender<AgentEvent>,
        interactive_policy_rx: &mut Option<mpsc::Receiver<InteractivePolicyReply>>,
        question_answer_rx: &mut Option<mpsc::Receiver<String>>,
        interrupt_after_current_tools: Option<Arc<AtomicBool>>,
    ) -> Result<(), AgentError> {
        self.prepare_for_new_user_turn()?;

        if !matches!(self.state, AgentState::Idle) {
            return Err(AgentError::SessionFailed {
                message: "AgentSession::run expected Idle state after prepare".into(),
            });
        }

        emit_user_turn(&self.journal, &task)?;

        let mut iteration: u32 = 0;
        let mut user_line_committed = false;

        'session: loop {
            match &self.state {
                AgentState::Complete => return Ok(()),
                AgentState::Failed { error, .. } => return Err(error.clone()),
                _ => {}
            }

            check_iteration_limit(iteration, &self.config)?;

            if self.budget_stop_before_next_iteration {
                self.apply_event(
                    &event_tx,
                    AgentEvent::StatusInfo {
                        message:
                            "Headless budget limit reached — stopping before another model call."
                                .into(),
                    },
                    &task,
                )
                .await?;
                self.apply_event(&event_tx, AgentEvent::Done, &task).await?;
                self.last_run_exit = SessionRunExit::BudgetLimit;
                self.record_run_finished_success();
                return Ok(());
            }

            let keep_tail = if self.provider.name().to_lowercase().starts_with("groq/") {
                MICROCOMPACT_KEEP_RECENT_GROQ
            } else {
                MICROCOMPACT_KEEP_RECENT_DEFAULT
            };
            let est_cleared = apply_microcompact_context(&mut self.context, keep_tail);
            if est_cleared > 0 {
                tracing::debug!("microcompact: ~{} tokens cleared", est_cleared);
                self.apply_event(
                    &event_tx,
                    AgentEvent::MicrocompactEstimate {
                        estimated_tokens_cleared: est_cleared,
                    },
                    &task,
                )
                .await?;
            }

            if matches!(self.state, AgentState::Idle) {
                self.apply_event(
                    &event_tx,
                    AgentEvent::IterationStarted {
                        n: iteration.saturating_add(1),
                        max: self.config.max_iterations,
                    },
                    &task,
                )
                .await?;
            }

            let project_root = self.sandbox.primary_root().display().to_string();
            let model_id = self.provider.completion_model_id().to_string();
            let tools_for_call = tools_for_model_id(model_id.as_str(), &self.tools);
            let tool_name_strings: Vec<String> = tools_for_call
                .iter()
                .map(|t| t.as_ref().name().to_string())
                .collect();
            let tool_names: Vec<&str> = tool_name_strings.iter().map(|s| s.as_str()).collect();
            let messages = self.compose_model_messages(
                task.as_str(),
                user_line_committed,
                project_root.as_str(),
                &tool_names,
                model_id.as_str(),
            );

            let mut messages = messages;
            let mut sum_round = 0u32;
            while sum_round < 8
                && self
                    .context_manager
                    .needs_summarization(&messages, self.provider.as_ref())
            {
                sum_round = sum_round.saturating_add(1);
                self.run_context_summarization_pass(
                    &messages,
                    &event_tx,
                    task.as_str(),
                    user_line_committed,
                )
                .await?;
                let tool_names: Vec<&str> = tool_name_strings.iter().map(|s| s.as_str()).collect();
                messages = self.compose_model_messages(
                    task.as_str(),
                    user_line_committed,
                    project_root.as_str(),
                    &tool_names,
                    model_id.as_str(),
                );
            }

            let before_trim_len = messages.len();
            let messages = trim_messages_for_model(model_id.as_str(), messages);
            if looks_like_ollama_model(model_id.as_str()) && messages.len() < before_trim_len {
                self.apply_event(
                    &event_tx,
                    AgentEvent::StatusInfo {
                        message: "Trimmed Ollama context to system + last 6 messages for faster first token."
                            .into(),
                    },
                    &task,
                )
                .await?;
            }

            if std::env::var_os("AKMON_DEBUG_CACHE").as_deref() == Some(std::ffi::OsStr::new("1")) {
                let sys = anthropic_system_block_text(&messages);
                eprintln!(
                    "akmon: debug cache model_call={} system_joined_len={}",
                    iteration.saturating_add(1),
                    sys.len()
                );
            }

            let completion_config = completion_config_for_tools(
                &tools_for_call,
                self.provider.as_ref(),
                self.config.session_id,
                self.config.max_completion_tokens,
                self.config.fallback_model.clone(),
            );
            self.capture_prompt_assembly_hash_if_absent(&messages, &completion_config);
            tracing::debug!(
                "tools for {}: {}",
                model_id.as_str(),
                completion_config.tools.len()
            );
            let mut stream: CompletionStream =
                match self.provider.complete(&messages, &completion_config).await {
                    Ok(s) => s,
                    Err(e) => match self.handle_model_error_for_run(&event_tx, &task, e).await? {
                        Some(ae) => return Err(ae),
                        None => return Ok(()),
                    },
                };

            let mut accumulated = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Err(e) => match self.handle_model_error_for_run(&event_tx, &task, e).await? {
                        Some(ae) => return Err(ae),
                        None => return Ok(()),
                    },
                    Ok(StreamEvent::ProviderReady { provider, model }) => {
                        self.apply_event(
                            &event_tx,
                            AgentEvent::ProviderConfirmed { provider, model },
                            &task,
                        )
                        .await?;
                    }
                    Ok(StreamEvent::StatusHint { message }) => {
                        self.apply_event(&event_tx, AgentEvent::StatusInfo { message }, &task)
                            .await?;
                    }
                    Ok(StreamEvent::TextDelta { text }) => {
                        accumulated.push_str(&text);
                        self.result_text.push_str(&text);
                        self.apply_event(&event_tx, AgentEvent::TextDelta { text }, &task)
                            .await?;
                    }
                    Ok(StreamEvent::UsageReport(r)) => {
                        self.apply_event(
                            &event_tx,
                            AgentEvent::UsageReport {
                                input_tokens: r.input_tokens,
                                output_tokens: r.output_tokens,
                                cache_creation_tokens: r.cache_creation_tokens,
                                cache_read_tokens: r.cache_read_tokens,
                            },
                            &task,
                        )
                        .await?;
                    }
                    Ok(StreamEvent::Error { error }) => {
                        match self
                            .handle_model_error_for_run(&event_tx, &task, error)
                            .await?
                        {
                            Some(ae) => return Err(ae),
                            None => return Ok(()),
                        }
                    }
                    Ok(StreamEvent::Done {
                        stop_reason,
                        tool_calls,
                    }) => {
                        if matches!(self.state, AgentState::Planning { .. }) {
                            self.apply_event(
                                &event_tx,
                                AgentEvent::TextDelta {
                                    text: String::new(),
                                },
                                &task,
                            )
                            .await?;
                        }

                        match stop_reason {
                            StopReason::MaxTokens => {
                                if !tool_calls.is_empty() {
                                    // Model was cut mid tool-call — execute what we have,
                                    // then continue. The tool results will be in context
                                    // for the next turn and the model can resume.
                                    if !user_line_committed {
                                        self.context.push(Message {
                                            role: MessageRole::User,
                                            content: task.clone(),
                                        });
                                        user_line_committed = true;
                                    }
                                    let assistant_record = json!({
                                        "text": &accumulated,
                                        "tool_calls": tool_calls.iter().map(|c| {
                                            json!({"id": &c.id, "name": &c.name, "arguments": &c.arguments})
                                        }).collect::<Vec<_>>(),
                                    });
                                    let tool_calls_cbor_opt =
                                        match crate::journal::assistant_tool_calls_cbor(&tool_calls)
                                        {
                                            Ok(b) => Some(b),
                                            Err(e) => {
                                                tracing::warn!(
                                                    target: "akmon::session",
                                                    error = %e,
                                                    "AssistantTurn tool_calls CBOR failed"
                                                );
                                                None
                                            }
                                        };
                                    self.context.push(Message {
                                        role: MessageRole::Assistant,
                                        content: assistant_record.to_string(),
                                    });
                                    self.dispatch_tool_calls_batch(
                                        tool_calls,
                                        &event_tx,
                                        &task,
                                        interactive_policy_rx,
                                        question_answer_rx,
                                    )
                                    .await?;
                                    self.warn_emit_assistant_turn_cbor(
                                        accumulated.as_str(),
                                        tool_calls_cbor_opt.as_deref(),
                                    );
                                    self.apply_event(
                                        &event_tx,
                                        AgentEvent::StatusInfo {
                                            message: "─ truncated mid-tool, resuming… ─".into(),
                                        },
                                        &task,
                                    )
                                    .await?;
                                    self.record_retry_attempt();
                                    iteration = iteration.saturating_add(1);
                                    continue 'session;
                                }

                                if self.continuation_count < 3 {
                                    self.continuation_count =
                                        self.continuation_count.saturating_add(1);
                                    self.record_retry_attempt();
                                    self.apply_event(
                                        &event_tx,
                                        AgentEvent::StatusInfo {
                                            message: format!(
                                                "─ response truncated, continuing ({}/3)… ─",
                                                self.continuation_count
                                            ),
                                        },
                                        &task,
                                    )
                                    .await?;

                                    if !user_line_committed {
                                        self.context.push(Message {
                                            role: MessageRole::User,
                                            content: task.clone(),
                                        });
                                        user_line_committed = true;
                                    }
                                    self.context.push(Message {
                                        role: MessageRole::Assistant,
                                        content: accumulated.clone(),
                                    });
                                    self.warn_emit_assistant_turn_cbor(accumulated.as_str(), None);
                                    self.context.push(Message {
                                        role: MessageRole::User,
                                        content: "Continue from exactly where you stopped. \
Do not repeat anything already written. \
Resume from the mid-sentence or mid-block point where the response was cut."
                                            .into(),
                                    });

                                    continue 'session;
                                }

                                self.apply_event(
                                    &event_tx,
                                    AgentEvent::StatusInfo {
                                        message: "─ response could not complete — try asking for a smaller piece at a time ─"
                                            .into(),
                                    },
                                    &task,
                                )
                                .await?;
                                self.continuation_count = 0;
                                let ae = AgentError::ResponseTruncated;
                                self.apply_event(
                                    &event_tx,
                                    AgentEvent::Error {
                                        error: ae.clone(),
                                        recoverable: false,
                                    },
                                    &task,
                                )
                                .await?;
                                return Err(ae);
                            }
                            StopReason::EndTurn => {
                                self.continuation_count = 0;
                                // If the model ended its turn with pending tool calls,
                                // execute them and continue. This happens when the model
                                // produces a text response AND tool calls in the same turn.
                                if !tool_calls.is_empty() {
                                    if !user_line_committed {
                                        self.context.push(Message {
                                            role: MessageRole::User,
                                            content: task.clone(),
                                        });
                                        user_line_committed = true;
                                    }
                                    let assistant_record = json!({
                                        "text": &accumulated,
                                        "tool_calls": tool_calls.iter().map(|c| {
                                            json!({"id": &c.id, "name": &c.name, "arguments": &c.arguments})
                                        }).collect::<Vec<_>>(),
                                    });
                                    let tool_calls_cbor_opt =
                                        match crate::journal::assistant_tool_calls_cbor(&tool_calls)
                                        {
                                            Ok(b) => Some(b),
                                            Err(e) => {
                                                tracing::warn!(
                                                    target: "akmon::session",
                                                    error = %e,
                                                    "AssistantTurn tool_calls CBOR failed"
                                                );
                                                None
                                            }
                                        };
                                    self.context.push(Message {
                                        role: MessageRole::Assistant,
                                        content: assistant_record.to_string(),
                                    });
                                    self.dispatch_tool_calls_batch(
                                        tool_calls,
                                        &event_tx,
                                        &task,
                                        interactive_policy_rx,
                                        question_answer_rx,
                                    )
                                    .await?;
                                    self.warn_emit_assistant_turn_cbor(
                                        accumulated.as_str(),
                                        tool_calls_cbor_opt.as_deref(),
                                    );
                                    if interrupt_after_current_tools
                                        .as_ref()
                                        .is_some_and(|f| f.load(Ordering::SeqCst))
                                    {
                                        self.apply_event(&event_tx, AgentEvent::Done, &task)
                                            .await?;
                                        self.record_run_finished_success();
                                        self.last_run_exit =
                                            if self.budget_stop_before_next_iteration {
                                                SessionRunExit::BudgetLimit
                                            } else {
                                                SessionRunExit::Completed
                                            };
                                        return Ok(());
                                    }
                                    iteration = iteration.saturating_add(1);
                                    continue 'session;
                                }

                                // No tool calls — model is genuinely done with the task.
                                self.apply_event(&event_tx, AgentEvent::Done, &task).await?;
                                if !user_line_committed {
                                    self.context.push(Message {
                                        role: MessageRole::User,
                                        content: task.clone(),
                                    });
                                }
                                self.context.push(Message {
                                    role: MessageRole::Assistant,
                                    content: accumulated.clone(),
                                });
                                self.warn_emit_assistant_turn_cbor(accumulated.as_str(), None);
                                self.record_run_finished_success();
                                self.last_run_exit = if self.budget_stop_before_next_iteration {
                                    SessionRunExit::BudgetLimit
                                } else {
                                    SessionRunExit::Completed
                                };
                                return Ok(());
                            }
                            StopReason::ToolUse => {
                                self.continuation_count = 0;
                                if tool_calls.is_empty() {
                                    let ae = AgentError::ModelError {
                                        message: "model returned ToolUse with no tool_calls".into(),
                                    };
                                    self.apply_event(
                                        &event_tx,
                                        AgentEvent::Error {
                                            error: ae.clone(),
                                            recoverable: false,
                                        },
                                        &task,
                                    )
                                    .await?;
                                    return Err(ae);
                                }

                                if !user_line_committed {
                                    self.context.push(Message {
                                        role: MessageRole::User,
                                        content: task.clone(),
                                    });
                                    user_line_committed = true;
                                }

                                let assistant_record = json!({
                                    "text": &accumulated,
                                    "tool_calls": tool_calls.iter().map(|c| {
                                        json!({"id": &c.id, "name": &c.name, "arguments": &c.arguments})
                                    }).collect::<Vec<_>>(),
                                });
                                let tool_calls_cbor_opt =
                                    match crate::journal::assistant_tool_calls_cbor(&tool_calls) {
                                        Ok(b) => Some(b),
                                        Err(e) => {
                                            tracing::warn!(
                                                target: "akmon::session",
                                                error = %e,
                                                "AssistantTurn tool_calls CBOR failed"
                                            );
                                            None
                                        }
                                    };
                                self.context.push(Message {
                                    role: MessageRole::Assistant,
                                    content: assistant_record.to_string(),
                                });

                                self.dispatch_tool_calls_batch(
                                    tool_calls,
                                    &event_tx,
                                    &task,
                                    interactive_policy_rx,
                                    question_answer_rx,
                                )
                                .await?;
                                self.warn_emit_assistant_turn_cbor(
                                    accumulated.as_str(),
                                    tool_calls_cbor_opt.as_deref(),
                                );

                                if interrupt_after_current_tools
                                    .as_ref()
                                    .is_some_and(|f| f.load(Ordering::SeqCst))
                                {
                                    self.apply_event(&event_tx, AgentEvent::Done, &task).await?;
                                    self.record_run_finished_success();
                                    self.last_run_exit = if self.budget_stop_before_next_iteration {
                                        SessionRunExit::BudgetLimit
                                    } else {
                                        SessionRunExit::Completed
                                    };
                                    return Ok(());
                                }

                                iteration = iteration.saturating_add(1);
                                continue 'session;
                            }
                        }
                    }
                }
            }

            match &self.state {
                AgentState::Complete => return Ok(()),
                AgentState::Failed { error, .. } => return Err(error.clone()),
                _ => {
                    let ae = AgentError::ModelError {
                        message: "completion stream ended before Done".into(),
                    };
                    self.apply_event(
                        &event_tx,
                        AgentEvent::Error {
                            error: ae.clone(),
                            recoverable: false,
                        },
                        &task,
                    )
                    .await?;
                    return Err(ae);
                }
            }
        }
    }

    fn capture_prompt_assembly_hash_if_absent(
        &mut self,
        messages: &[Message],
        completion_config: &CompletionConfig,
    ) {
        if self.prompt_assembly_fingerprint.is_some() {
            return;
        }
        let mut role_counts: HashMap<&'static str, u64> = HashMap::new();
        for m in messages {
            let key = match m.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "tool",
            };
            let entry = role_counts.entry(key).or_insert(0);
            *entry = entry.saturating_add(1);
        }
        let mut tool_names = completion_config
            .tools
            .iter()
            .map(|d| d.name.clone())
            .collect::<Vec<_>>();
        tool_names.sort();
        let fingerprint = json!({
            "message_count": messages.len(),
            "role_counts": role_counts,
            "message_char_lengths": messages.iter().map(|m| m.content.chars().count()).collect::<Vec<usize>>(),
            "tool_names": tool_names,
            "max_tokens": completion_config.max_tokens,
            "temperature": completion_config.temperature,
            "first_token_deadline_ms": completion_config.first_token_deadline_ms,
            "stream": completion_config.stream,
            "has_fallback_model": completion_config.fallback_model.is_some(),
        });
        self.prompt_assembly_fingerprint = Some(fingerprint);
        self.refresh_replay_metadata();
    }

    fn refresh_replay_metadata(&mut self) {
        let policy_value = match serde_json::to_value(self.policy.mode()) {
            Ok(v) => v,
            Err(e) => json!({ "serialization_error": e.to_string() }),
        };
        // Intentionally hashes only non-secret runtime knobs. Provider credentials and tokens
        // are excluded from replay metadata by design.
        let config_value = json!({
            "max_iterations": self.config.max_iterations,
            "confirmation_timeout_secs": self.config.confirmation_timeout_secs,
            "auto_commit": self.config.auto_commit,
            "max_completion_tokens": self.config.max_completion_tokens,
            "subagent_style": self.config.subagent_style,
            "max_budget_usd": self.config.max_budget_usd,
            "fallback_model": self.config.fallback_model,
            "model_estimates": self.config.model_estimates,
            "plan_mode": self.plan_mode,
        });
        let mut tool_registry = self
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "required_permissions": t.required_permissions(),
                    "parameters_schema": t.parameters_schema(),
                })
            })
            .collect::<Vec<Value>>();
        tool_registry.sort_by(|a, b| {
            let an = a
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let bn = b
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            an.cmp(&bn)
        });
        let inputs = ReplayHashInputs {
            policy: policy_value,
            config: config_value,
            tool_registry: Value::Array(tool_registry),
            prompt_assembly: self.prompt_assembly_fingerprint.clone(),
        };
        let session_id = self.config.session_id.to_string();
        match build_replay_metadata(
            self.provider.name(),
            self.provider.completion_model_id(),
            session_id.as_str(),
            &inputs,
        ) {
            Ok(m) => {
                self.replay_metadata = Some(m);
            }
            Err(e) => {
                tracing::warn!("failed to build replay metadata: {e}");
                self.replay_metadata = None;
            }
        }
    }

    /// Policy + optional parallel execution for one model turn's tool calls.
    async fn dispatch_tool_calls_batch(
        &mut self,
        tool_calls: Vec<ModelToolCall>,
        event_tx: &mpsc::Sender<AgentEvent>,
        task: &str,
        interactive_policy_rx: &mut Option<mpsc::Receiver<InteractivePolicyReply>>,
        question_answer_rx: &mut Option<mpsc::Receiver<String>>,
    ) -> Result<(), AgentError> {
        let n = tool_calls.len();
        let mut slots: Vec<Option<ToolCallResult>> = vec![None; n];

        struct ApprovedSlot {
            original_index: usize,
            id: String,
            name: String,
            arguments: Value,
            tool_idx: usize,
            mcp_context: Option<McpAuditContext>,
        }
        let mut approved: Vec<ApprovedSlot> = Vec::new();
        let mut write_file_calls_this_message: u32 = 0;

        for (idx, call) in tool_calls.iter().enumerate() {
            let id = call.id.clone();
            let name = call.name.clone();
            let args = call.arguments.clone();

            let matching_tool_indices: Vec<usize> = self
                .tools
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (t.as_ref().name() == name).then_some(i))
                .collect();
            let Some(tool_idx) = matching_tool_indices.first().copied() else {
                let msg = format!("tool not found: {name}");
                self.apply_event(
                    event_tx,
                    AgentEvent::ToolCallCompleted {
                        id: id.clone(),
                        name: name.clone(),
                        success: false,
                        message: msg.clone(),
                    },
                    task,
                )
                .await?;
                slots[idx] = Some(ToolCallResult {
                    call_id: id.clone(),
                    tool_name: name.clone(),
                    output: ToolOutput::Error {
                        code: akmon_tools::ToolErrorCode::InvalidArgs,
                        message: msg,
                    },
                    success: false,
                    arguments: args.clone(),
                    latency_ms: 0,
                });
                continue;
            };
            if matching_tool_indices.len() > 1 {
                let matching_mcp_contexts: Vec<McpPolicyContext> = matching_tool_indices
                    .iter()
                    .filter_map(|i| self.tools[*i].mcp_policy_context())
                    .collect();
                if matching_mcp_contexts.len() > 1 {
                    let msg = format!(
                        "policy denied for tool `{name}`: ambiguous MCP context (multiple servers expose the same tool name)"
                    );
                    self.record_policy_denial();
                    self.apply_event(
                        event_tx,
                        AgentEvent::ToolCallCompleted {
                            id: id.clone(),
                            name: name.clone(),
                            success: false,
                            message: msg.clone(),
                        },
                        task,
                    )
                    .await?;
                    slots[idx] = Some(ToolCallResult {
                        call_id: id.clone(),
                        tool_name: name.clone(),
                        output: ToolOutput::Error {
                            code: akmon_tools::ToolErrorCode::PermissionDenied,
                            message: msg,
                        },
                        success: false,
                        arguments: args.clone(),
                        latency_ms: 0,
                    });
                    continue;
                }
            }

            const MAX_WRITE_FILE_CALLS_PER_ASSISTANT_MESSAGE: u32 = 2;
            if name == "write_file" {
                write_file_calls_this_message += 1;
                if write_file_calls_this_message > MAX_WRITE_FILE_CALLS_PER_ASSISTANT_MESSAGE {
                    let msg = "Only two write_file calls per assistant message, please. \
Complete and verify the current file(s), then continue in the next turn.";
                    self.apply_event(
                        event_tx,
                        AgentEvent::ToolCallCompleted {
                            id: id.clone(),
                            name: name.clone(),
                            success: false,
                            message: msg.to_string(),
                        },
                        task,
                    )
                    .await?;
                    slots[idx] = Some(ToolCallResult {
                        call_id: id.clone(),
                        tool_name: name.clone(),
                        output: ToolOutput::Error {
                            code: akmon_tools::ToolErrorCode::InvalidArgs,
                            message: msg.to_string(),
                        },
                        success: false,
                        arguments: args.clone(),
                        latency_ms: 0,
                    });
                    continue;
                }
            }

            const TOOL_REPEAT_LIMIT: u32 = 5;
            if (name == "list_directory" || name == "read_file")
                && self
                    .tool_call_counts
                    .get(name.as_str())
                    .copied()
                    .unwrap_or(0)
                    >= TOOL_REPEAT_LIMIT
            {
                let call_count = self
                    .tool_call_counts
                    .get(name.as_str())
                    .copied()
                    .unwrap_or(0);
                let msg = format!(
                    "You have called {name} {call_count} times already. \
                     Stop exploring and start building. \
                     Use write_file to create the files needed \
                     for this task. Make your best attempt now."
                );
                self.apply_event(
                    event_tx,
                    AgentEvent::ToolCallCompleted {
                        id: id.clone(),
                        name: name.clone(),
                        // false: the tool was never dispatched (still in Thinking state).
                        // Thinking + ToolCallCompleted { success: true } is an illegal FSM
                        // transition; success: false is the correct shape for inline rejections
                        // that short-circuit before ToolExecution.
                        success: false,
                        message: msg.clone(),
                    },
                    task,
                )
                .await?;
                self.apply_event(
                    event_tx,
                    AgentEvent::StatusInfo {
                        message: format!(
                            "─ {name} called {call_count} times, nudging agent to act ─"
                        ),
                    },
                    task,
                )
                .await?;
                slots[idx] = Some(ToolCallResult {
                    call_id: id.clone(),
                    tool_name: name.clone(),
                    output: ToolOutput::Success { content: msg },
                    success: true,
                    arguments: args.clone(),
                    latency_ms: 0,
                });
                continue;
            }

            // Resolve policy before dispatch so confirmations stay in Thinking and parallel
            // batches do not emit completions while `parallel_tool_batch_remaining` is unset.
            let perms = concrete_permissions(
                self.tools[tool_idx].as_ref(),
                &name,
                &args,
                self.sandbox.primary_root(),
            );
            let session_id = self.config.session_id.to_string();
            let mut policy_denial_message: Option<String> = None;
            let diff_preview =
                file_change_diff_preview(self.sandbox.as_ref(), name.as_str(), &args).await;
            let mcp_policy_context =
                self.tools[tool_idx]
                    .mcp_policy_context()
                    .map(|ctx| McpAuditContext {
                        server: ctx.server,
                        tool: ctx.tool,
                    });
            if let Some(ctx) = &mcp_policy_context {
                let decision = self.policy.evaluate_mcp_automatic(
                    session_id.as_str(),
                    Some(name.as_str()),
                    Some(ctx.server.as_str()),
                    Some(ctx.tool.as_str()),
                );
                self.audit_log.push(decision.audit.clone());
                self.warn_emit_permission_gate(
                    "mcp:auto",
                    if decision.allowed {
                        "allowed"
                    } else {
                        "denied"
                    },
                    name.as_str(),
                    &args,
                    "evaluate_mcp_automatic (MCP server/tool context)",
                );
                if !decision.allowed {
                    policy_denial_message = Some(format!(
                        "policy denied for MCP tool `{name}` (server=`{}`, tool=`{}`): {}",
                        ctx.server, ctx.tool, decision.reason
                    ));
                }
            } else if name == "mcp_tool" {
                let decision = self.policy.evaluate_mcp_automatic(
                    session_id.as_str(),
                    Some(name.as_str()),
                    None,
                    None,
                );
                self.audit_log.push(decision.audit.clone());
                self.warn_emit_permission_gate(
                    "mcp:auto",
                    if decision.allowed {
                        "allowed"
                    } else {
                        "denied"
                    },
                    name.as_str(),
                    &args,
                    "evaluate_mcp_automatic (mcp_tool without MCP context)",
                );
                if !decision.allowed {
                    policy_denial_message = Some(format!(
                        "policy denied for MCP tool `{name}` due to malformed context: {}",
                        decision.reason
                    ));
                }
            }
            if policy_denial_message.is_some() {
                let deny_message = policy_denial_message
                    .clone()
                    .unwrap_or_else(|| "policy denied".to_string());
                self.record_policy_denial();
                self.apply_event(
                    event_tx,
                    AgentEvent::ToolCallCompleted {
                        id: id.clone(),
                        name: name.clone(),
                        success: false,
                        message: deny_message.clone(),
                    },
                    task,
                )
                .await?;
                slots[idx] = Some(ToolCallResult {
                    call_id: id.clone(),
                    tool_name: name.clone(),
                    output: ToolOutput::Error {
                        code: akmon_tools::ToolErrorCode::PermissionDenied,
                        message: deny_message,
                    },
                    success: false,
                    arguments: args.clone(),
                    latency_ms: 0,
                });
                continue;
            }

            for perm in perms {
                if self.permission_allow_all_writes && matches!(perm, Permission::WriteFile { .. })
                {
                    let decision = self.policy.resolve_interactive(
                        session_id.as_str(),
                        perm.clone(),
                        PolicyVerdict::Allow,
                        "allow all writes for session",
                    );
                    let decision = match decision {
                        Ok(d) => d,
                        Err(e) => {
                            return Err(AgentError::SessionFailed {
                                message: e.to_string(),
                            });
                        }
                    };
                    self.audit_log.push(decision.audit.clone());
                    self.warn_emit_permission_gate(
                        "remembered:write",
                        if decision.allowed {
                            "allowed"
                        } else {
                            "denied"
                        },
                        name.as_str(),
                        &args,
                        "allow-all-writes session shortcut (resolve_interactive Allow)",
                    );
                    if !decision.allowed {
                        policy_denial_message = Some(format!(
                            "policy denied for tool `{name}` permission `{perm:?}`: {}",
                            decision.reason
                        ));
                        break;
                    }
                    continue;
                }
                if let Permission::ExecuteCommand { command, .. } = &perm
                    && self
                        .permission_shell_prefixes
                        .iter()
                        .any(|pfx| command.starts_with(pfx))
                {
                    let decision = self.policy.resolve_interactive(
                        session_id.as_str(),
                        perm.clone(),
                        PolicyVerdict::Allow,
                        "shell command prefix allowed for session",
                    );
                    let decision = match decision {
                        Ok(d) => d,
                        Err(e) => {
                            return Err(AgentError::SessionFailed {
                                message: e.to_string(),
                            });
                        }
                    };
                    self.audit_log.push(decision.audit.clone());
                    self.warn_emit_permission_gate(
                        "remembered:shell-prefix",
                        if decision.allowed {
                            "allowed"
                        } else {
                            "denied"
                        },
                        name.as_str(),
                        &args,
                        "shell command prefix session shortcut (resolve_interactive Allow)",
                    );
                    if !decision.allowed {
                        policy_denial_message = Some(format!(
                            "policy denied for tool `{name}` permission `{perm:?}`: {}",
                            decision.reason
                        ));
                        break;
                    }
                    continue;
                }
                if self.permission_session_allowlist.contains(&perm) {
                    let decision = self.policy.resolve_interactive(
                        session_id.as_str(),
                        perm.clone(),
                        PolicyVerdict::Allow,
                        "session remembered approval for identical permission",
                    );
                    let decision = match decision {
                        Ok(d) => d,
                        Err(e) => {
                            return Err(AgentError::SessionFailed {
                                message: e.to_string(),
                            });
                        }
                    };
                    self.audit_log.push(decision.audit.clone());
                    self.warn_emit_permission_gate(
                        "remembered:session",
                        if decision.allowed {
                            "allowed"
                        } else {
                            "denied"
                        },
                        name.as_str(),
                        &args,
                        "session allowlist identical permission (resolve_interactive Allow)",
                    );
                    if !decision.allowed {
                        policy_denial_message = Some(format!(
                            "policy denied for tool `{name}` permission `{perm:?}`: {}",
                            decision.reason
                        ));
                        break;
                    }
                    continue;
                }

                let decision = match self.policy.mode() {
                    PolicyEngineMode::Interactive
                    | PolicyEngineMode::AutoApproveReads { .. }
                    | PolicyEngineMode::AutoApproveReadsAndFetch { .. } => {
                        match self.policy.evaluate_automatic_for_tool(
                            session_id.as_str(),
                            perm.clone(),
                            Some(name.as_str()),
                        ) {
                            Ok(decision) => {
                                self.audit_log.push(decision.audit.clone());
                                self.warn_emit_permission_gate(
                                    "tool:auto",
                                    if decision.allowed {
                                        "allowed"
                                    } else {
                                        "denied"
                                    },
                                    name.as_str(),
                                    &args,
                                    "evaluate_automatic_for_tool (interactive or auto-read modes)",
                                );
                                decision
                            }
                            Err(PolicyEngineError::InteractiveRequiresCaller) => {
                                let desc = match &perm {
                                    Permission::ExecuteCommand { command, cwd } => {
                                        format!(
                                            "Shell command requires confirmation.\n  Proposed command:\n    {command}\n  Working directory: {}",
                                            cwd.display()
                                        )
                                    }
                                    Permission::NetworkFetch { url } => {
                                        format!(
                                            "Network fetch requires confirmation.\n  URL: {url}"
                                        )
                                    }
                                    Permission::WriteFile { path } => {
                                        format!(
                                            "File change requires confirmation.\n  Path: {}",
                                            path.display()
                                        )
                                    }
                                    _ => format!("Permission required: {perm:?}"),
                                };
                                self.apply_event(
                                    event_tx,
                                    AgentEvent::ConfirmationRequired {
                                        description: desc.clone(),
                                        diff_preview: diff_preview.clone(),
                                    },
                                    task,
                                )
                                .await?;
                                let Some(rx) = interactive_policy_rx.as_mut() else {
                                    return Err(AgentError::SessionFailed {
                                        message: "interactive policy requires a verdict channel"
                                            .into(),
                                    });
                                };
                                let reply = match rx.recv().await {
                                    Some(v) => v,
                                    None => {
                                        return Err(AgentError::SessionFailed {
                                            message: "policy verdict channel closed".into(),
                                        });
                                    }
                                };
                                let reason: String = match reply.verdict {
                                    PolicyVerdict::Allow => {
                                        if reply.remember_for_session {
                                            "user approved; remembered for this session".into()
                                        } else {
                                            "user approved (interactive)".into()
                                        }
                                    }
                                    PolicyVerdict::Deny => "user denied (interactive)".into(),
                                };
                                let decision = self.policy.resolve_interactive(
                                    session_id.as_str(),
                                    perm.clone(),
                                    reply.verdict,
                                    reason,
                                );
                                let decision = match decision {
                                    Ok(d) => d,
                                    Err(e) => {
                                        return Err(AgentError::SessionFailed {
                                            message: e.to_string(),
                                        });
                                    }
                                };
                                self.audit_log.push(decision.audit.clone());
                                self.warn_emit_permission_gate(
                                    "interactive",
                                    if decision.allowed {
                                        "allowed"
                                    } else {
                                        "denied"
                                    },
                                    name.as_str(),
                                    &args,
                                    "resolve_interactive after user confirmation",
                                );
                                if reply.remember_for_session
                                    && decision.allowed
                                    && !self.permission_session_allowlist.contains(&perm)
                                {
                                    self.permission_session_allowlist.push(perm.clone());
                                }
                                if reply.allow_all_writes_session && decision.allowed {
                                    self.permission_allow_all_writes = true;
                                }
                                if decision.allowed
                                    && let Some(prefix) = reply.shell_allow_prefix.clone()
                                    && !prefix.trim().is_empty()
                                {
                                    self.permission_shell_prefixes.push(prefix);
                                }
                                self.apply_event(
                                    event_tx,
                                    AgentEvent::TextDelta {
                                        text: String::new(),
                                    },
                                    task,
                                )
                                .await?;
                                decision
                            }
                            Err(e) => {
                                return Err(AgentError::SessionFailed {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                    _ => {
                        let decision = match self.policy.evaluate_automatic_for_tool(
                            session_id.as_str(),
                            perm.clone(),
                            Some(name.as_str()),
                        ) {
                            Ok(d) => d,
                            Err(e) => {
                                return Err(AgentError::SessionFailed {
                                    message: e.to_string(),
                                });
                            }
                        };
                        self.audit_log.push(decision.audit.clone());
                        self.warn_emit_permission_gate(
                            "tool:auto",
                            if decision.allowed {
                                "allowed"
                            } else {
                                "denied"
                            },
                            name.as_str(),
                            &args,
                            "evaluate_automatic_for_tool (configured or deny-all mode)",
                        );
                        decision
                    }
                };

                if !decision.allowed {
                    policy_denial_message = Some(format!(
                        "policy denied for tool `{name}` permission `{perm:?}`: {}",
                        decision.reason
                    ));
                    break;
                }
            }

            if let Some(msg) = policy_denial_message {
                self.record_policy_denial();
                self.apply_event(
                    event_tx,
                    AgentEvent::ToolCallCompleted {
                        id: id.clone(),
                        name: name.clone(),
                        success: false,
                        message: msg.clone(),
                    },
                    task,
                )
                .await?;
                slots[idx] = Some(ToolCallResult {
                    call_id: id.clone(),
                    tool_name: name.clone(),
                    output: ToolOutput::Error {
                        code: akmon_tools::ToolErrorCode::PermissionDenied,
                        message: msg,
                    },
                    success: false,
                    arguments: args.clone(),
                    latency_ms: 0,
                });
                continue;
            }

            approved.push(ApprovedSlot {
                original_index: idx,
                id,
                name,
                arguments: args,
                tool_idx,
                mcp_context: mcp_policy_context,
            });
        }

        for a in &approved {
            if let Some(ctx) = &a.mcp_context {
                self.mcp_call_context_by_id
                    .insert(a.id.clone(), ctx.clone());
            }
            self.apply_event(
                event_tx,
                AgentEvent::ToolCallDispatched {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    arguments: a.arguments.clone(),
                },
                task,
            )
            .await?;
        }

        if !approved.is_empty() {
            let batch_n = approved.len() as u32;
            self.parallel_tool_batch_remaining = batch_n;

            let sandbox = Arc::clone(&self.sandbox);
            let policy = Arc::clone(&self.policy);
            let session_id = self.config.session_id;
            let interactive_tools = question_answer_rx.is_some();

            let mut unordered: FuturesUnordered<_> = FuturesUnordered::new();
            for a in approved {
                let tool = Arc::clone(&self.tools[a.tool_idx]);
                let args = a.arguments;
                let orig = a.original_index;
                let pending = PendingToolCall {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    arguments: args,
                };
                let sandbox_c = Arc::clone(&sandbox);
                let policy_c = Arc::clone(&policy);
                unordered.push(async move {
                    let ctx = ToolContext::new((*sandbox_c).clone(), policy_c)
                        .with_session(session_id, interactive_tools);
                    let result = execute_single_tool_call(&pending, tool.as_ref(), &ctx).await;
                    (orig, result)
                });
            }

            let mut collected: Vec<(usize, ToolCallResult)> = Vec::new();
            while let Some(pair) = unordered.next().await {
                collected.push(pair);
            }
            collected.sort_by_key(|(i, _)| *i);

            for (orig_idx, mut tool_result) in collected {
                *self
                    .tool_call_counts
                    .entry(tool_result.tool_name.clone())
                    .or_insert(0) += 1;

                if let ToolOutput::Question {
                    question,
                    suggestions,
                } = &tool_result.output
                {
                    let q = question.clone();
                    let sug = suggestions.clone();
                    if let Some(rx) = question_answer_rx.as_mut() {
                        self.apply_event(
                            event_tx,
                            AgentEvent::QuestionRequired {
                                id: tool_result.call_id.clone(),
                                question: q,
                                suggestions: sug,
                            },
                            task,
                        )
                        .await?;
                        let answer = match rx.recv().await {
                            Some(a) => a,
                            None => {
                                return Err(AgentError::SessionFailed {
                                    message: "question answer channel closed".into(),
                                });
                            }
                        };
                        tool_result.output = ToolOutput::Success { content: answer };
                        tool_result.success = true;
                    } else {
                        tool_result.output = ToolOutput::Error {
                            code: akmon_tools::ToolErrorCode::PermissionDenied,
                            message: "ask_followup requires an interactive session with a TUI answer channel"
                                .into(),
                        };
                        tool_result.success = false;
                    }
                }

                self.note_successful_file_tool_for_handoff(&tool_result);
                let message = match &tool_result.output {
                    ToolOutput::Success { content } => content.clone(),
                    ToolOutput::Error { message, .. } => message.clone(),
                    ToolOutput::Question { .. } => {
                        return Err(AgentError::SessionFailed {
                            message: "internal: unresolved Question tool output".into(),
                        });
                    }
                };
                self.apply_event(
                    event_tx,
                    AgentEvent::ToolCallCompleted {
                        id: tool_result.call_id.clone(),
                        name: tool_result.tool_name.clone(),
                        success: tool_result.success,
                        message,
                    },
                    task,
                )
                .await?;
                if self.config.auto_commit
                    && self.sandbox.has_git_root
                    && tool_result.success
                    && let Some(ev) = akmon_tools::try_auto_commit_after_file_tool(
                        self.sandbox.primary_root(),
                        &self.config.session_id.to_string(),
                        &tool_result.tool_name,
                        &tool_result.arguments,
                    )
                {
                    self.audit_log.push(ev);
                }
                slots[orig_idx] = Some(tool_result);
            }
        }

        for slot in slots.iter().take(n) {
            let r = match slot {
                Some(v) => v,
                None => {
                    return Err(AgentError::SessionFailed {
                        message: "internal: missing tool result slot".into(),
                    });
                }
            };
            self.record_tool_completion_metrics(r);
            self.append_tool_message(&r.call_id, &r.tool_name, r.output.clone())?;
        }

        Ok(())
    }

    fn append_tool_message(
        &mut self,
        tool_call_id: &str,
        tool_name: &str,
        output: ToolOutput,
    ) -> Result<(), AgentError> {
        let payload = json!({
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "output": output,
        });
        let content = serde_json::to_string(&payload).map_err(|e| AgentError::SessionFailed {
            message: format!("tool result json: {e}"),
        })?;
        self.context.push(Message {
            role: MessageRole::Tool,
            content,
        });
        Ok(())
    }

    /// Runs one compaction: model-summary of old turns, then folds them into [`Self::context`].
    async fn run_context_summarization_pass(
        &mut self,
        messages_full: &[Message],
        event_tx: &mpsc::Sender<AgentEvent>,
        task: &str,
        user_line_committed: bool,
    ) -> Result<(), AgentError> {
        let main_end = if !user_line_committed {
            messages_full.len().saturating_sub(1)
        } else {
            messages_full.len()
        };
        let head = &messages_full[..main_end];
        let (to_s, _to_k) = self.context_manager.messages_to_summarize(head);
        if to_s.is_empty() {
            // Do not eprintln here: the interactive TUI uses the same terminal as stderr, and a
            // line on every turn corrupts the layout (text appears over the compose area).
            self.trim_oldest_non_system_fraction_of_context(0.2);
            return Ok(());
        }

        let resume = match &self.state {
            AgentState::Planning { task: t, iteration } => AgentState::Planning {
                task: t.clone(),
                iteration: *iteration,
            },
            AgentState::Thinking { iteration } => AgentState::Thinking {
                iteration: *iteration,
            },
            _ => {
                return Err(AgentError::SessionFailed {
                    message: "context summarization requires Planning or Thinking state".into(),
                });
            }
        };
        self.post_summary_resume = Some(resume);
        self.apply_event(event_tx, AgentEvent::SummarizationStarted, task)
            .await?;

        let tokens_before = approximate_tokens(to_s);
        let instruct = "Summarize the following conversation history concisely. Preserve key decisions, file paths mentioned, and technical context. Output only the summary, no preamble.";
        let mut summary_msgs = vec![Message {
            role: MessageRole::System,
            content: instruct.into(),
        }];
        summary_msgs.extend(to_s.iter().cloned());

        let sum_config = CompletionConfig {
            tools: Vec::new(),
            max_tokens: max_tokens_for_model(self.provider.completion_model_id()),
            ..CompletionConfig::default()
        };

        let mut stream = match self.provider.complete(&summary_msgs, &sum_config).await {
            Ok(s) => s,
            Err(e) => {
                if matches!(e, ModelError::RateLimited { .. }) {
                    match self.handle_model_error_for_run(event_tx, task, e).await? {
                        Some(ae) => return Err(ae),
                        None => return Ok(()),
                    }
                }
                self.restore_state_after_summarization_abort();
                self.trim_oldest_non_system_fraction_of_context(0.2);
                return Ok(());
            }
        };

        let mut summary_text = String::new();
        loop {
            let Some(item) = stream.next().await else {
                if summary_text.is_empty() {
                    self.restore_state_after_summarization_abort();
                    self.trim_oldest_non_system_fraction_of_context(0.2);
                    return Ok(());
                }
                break;
            };
            match item {
                Err(e) => {
                    if matches!(e, ModelError::RateLimited { .. }) {
                        match self.handle_model_error_for_run(event_tx, task, e).await? {
                            Some(ae) => return Err(ae),
                            None => return Ok(()),
                        }
                    }
                    self.restore_state_after_summarization_abort();
                    self.trim_oldest_non_system_fraction_of_context(0.2);
                    return Ok(());
                }
                Ok(StreamEvent::TextDelta { text }) => summary_text.push_str(&text),
                Ok(StreamEvent::UsageReport(_)) => {}
                Ok(StreamEvent::ProviderReady { .. }) => {}
                Ok(StreamEvent::StatusHint { .. }) => {}
                Ok(StreamEvent::Error { error }) => {
                    if matches!(error, ModelError::RateLimited { .. }) {
                        match self
                            .handle_model_error_for_run(event_tx, task, error)
                            .await?
                        {
                            Some(ae) => return Err(ae),
                            None => return Ok(()),
                        }
                    }
                    self.restore_state_after_summarization_abort();
                    self.trim_oldest_non_system_fraction_of_context(0.2);
                    return Ok(());
                }
                Ok(StreamEvent::Done {
                    stop_reason,
                    tool_calls,
                }) => {
                    if matches!(stop_reason, StopReason::MaxTokens) {
                        self.restore_state_after_summarization_abort();
                        self.trim_oldest_non_system_fraction_of_context(0.2);
                        return Ok(());
                    }
                    if !tool_calls.is_empty() {
                        self.restore_state_after_summarization_abort();
                        self.trim_oldest_non_system_fraction_of_context(0.2);
                        return Ok(());
                    }
                    break;
                }
            }
        }

        let summary_msg = Message {
            role: MessageRole::System,
            content: format!("<<<SUMMARY_START>>>\n{summary_text}\n<<<SUMMARY_END>>>"),
        };
        let tokens_after = approximate_tokens(std::slice::from_ref(&summary_msg));
        let tokens_freed = tokens_before.saturating_sub(tokens_after);

        if self
            .apply_folded_summary_to_context(to_s, head, summary_msg)
            .is_err()
        {
            self.restore_state_after_summarization_abort();
            self.trim_oldest_non_system_fraction_of_context(0.2);
            return Ok(());
        }

        self.apply_event(
            event_tx,
            AgentEvent::ContextSummarized {
                messages_replaced: to_s.len(),
                tokens_freed,
            },
            task,
        )
        .await?;
        Ok(())
    }

    fn apply_folded_summary_to_context(
        &mut self,
        to_s: &[Message],
        head: &[Message],
        summary: Message,
    ) -> Result<(), AgentError> {
        let n_fixed = self.context_manager.fixed_system_messages.min(head.len());
        let body = &head[n_fixed..];
        if body.len() != self.context.len() {
            return Err(AgentError::SessionFailed {
                message: "summary fold: context length mismatch".into(),
            });
        }
        let mut j = 0usize;
        while j < body.len() && body[j].role == MessageRole::System {
            j += 1;
        }
        if j + to_s.len() > self.context.len() {
            return Err(AgentError::SessionFailed {
                message: "summary fold: context shorter than expected".into(),
            });
        }
        if body.get(j..j + to_s.len()) != Some(to_s) {
            return Err(AgentError::SessionFailed {
                message: "summary fold: slice mismatch".into(),
            });
        }
        let suffix_from = j + to_s.len();
        let suffix = self.context[suffix_from..].to_vec();
        self.context.truncate(j);
        self.context.push(summary);
        self.context.extend(suffix);
        Ok(())
    }

    fn restore_state_after_summarization_abort(&mut self) {
        if let Some(s) = self.post_summary_resume.take() {
            self.state = s;
        }
    }

    fn trim_oldest_non_system_fraction_of_context(&mut self, frac: f64) {
        let non_sys: Vec<usize> = self
            .context
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role != MessageRole::System)
            .map(|(i, _)| i)
            .collect();
        let n_drop = ((non_sys.len() as f64) * frac).ceil() as usize;
        if n_drop == 0 {
            return;
        }
        let drop: std::collections::HashSet<usize> = non_sys.iter().take(n_drop).copied().collect();
        let mut idx = 0usize;
        self.context.retain(|_| {
            let this = idx;
            idx += 1;
            !drop.contains(&this)
        });
    }

    fn next_state_after(
        &mut self,
        event: &AgentEvent,
        task: &str,
    ) -> Result<AgentState, AgentError> {
        match (&self.state, event) {
            (AgentState::Idle, AgentEvent::IterationStarted { .. }) => Ok(AgentState::Planning {
                task: task.to_string(),
                iteration: 0,
            }),
            (AgentState::Planning { iteration, .. }, AgentEvent::TextDelta { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { iteration }, AgentEvent::TextDelta { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::Planning { task, iteration }, AgentEvent::UsageReport { .. }) => {
                Ok(AgentState::Planning {
                    task: task.clone(),
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { iteration }, AgentEvent::UsageReport { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::ToolExecution { iteration }, AgentEvent::UsageReport { .. }) => {
                Ok(AgentState::ToolExecution {
                    iteration: *iteration,
                })
            }
            (AgentState::Summarizing { iteration }, AgentEvent::UsageReport { .. }) => {
                Ok(AgentState::Summarizing {
                    iteration: *iteration,
                })
            }
            (AgentState::Planning { task, iteration }, AgentEvent::ProviderConfirmed { .. }) => {
                Ok(AgentState::Planning {
                    task: task.clone(),
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { iteration }, AgentEvent::ProviderConfirmed { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::Planning { task, iteration }, AgentEvent::StatusInfo { .. }) => {
                Ok(AgentState::Planning {
                    task: task.clone(),
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { iteration }, AgentEvent::StatusInfo { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::ToolExecution { iteration }, AgentEvent::StatusInfo { .. }) => {
                Ok(AgentState::ToolExecution {
                    iteration: *iteration,
                })
            }
            (AgentState::ToolExecution { iteration }, AgentEvent::QuestionRequired { .. }) => {
                Ok(AgentState::ToolExecution {
                    iteration: *iteration,
                })
            }
            (AgentState::Summarizing { iteration }, AgentEvent::StatusInfo { .. }) => {
                Ok(AgentState::Summarizing {
                    iteration: *iteration,
                })
            }
            (AgentState::Idle, AgentEvent::MicrocompactEstimate { .. }) => Ok(AgentState::Idle),
            (AgentState::Planning { task, iteration }, AgentEvent::MicrocompactEstimate { .. }) => {
                Ok(AgentState::Planning {
                    task: task.clone(),
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { iteration }, AgentEvent::MicrocompactEstimate { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::ToolExecution { iteration }, AgentEvent::MicrocompactEstimate { .. }) => {
                Ok(AgentState::ToolExecution {
                    iteration: *iteration,
                })
            }
            (AgentState::Summarizing { iteration }, AgentEvent::MicrocompactEstimate { .. }) => {
                Ok(AgentState::Summarizing {
                    iteration: *iteration,
                })
            }
            (
                AgentState::AwaitingConfirmation { iteration },
                AgentEvent::MicrocompactEstimate { .. },
            ) => Ok(AgentState::AwaitingConfirmation {
                iteration: *iteration,
            }),
            (AgentState::Thinking { .. }, AgentEvent::Done) => Ok(AgentState::Complete),
            (AgentState::Thinking { iteration }, AgentEvent::ToolCallDispatched { .. }) => {
                Ok(AgentState::ToolExecution {
                    iteration: *iteration,
                })
            }
            (AgentState::ToolExecution { iteration }, AgentEvent::ToolCallDispatched { .. }) => {
                Ok(AgentState::ToolExecution {
                    iteration: *iteration,
                })
            }
            (AgentState::ToolExecution { iteration }, AgentEvent::ToolCallCompleted { .. }) => {
                if self.parallel_tool_batch_remaining <= 1 {
                    self.parallel_tool_batch_remaining = 0;
                    Ok(AgentState::Thinking {
                        iteration: *iteration,
                    })
                } else {
                    self.parallel_tool_batch_remaining -= 1;
                    Ok(AgentState::ToolExecution {
                        iteration: *iteration,
                    })
                }
            }
            (
                AgentState::Thinking { iteration },
                AgentEvent::ToolCallCompleted { success: false, .. },
            ) => Ok(AgentState::Thinking {
                iteration: *iteration,
            }),
            (AgentState::Thinking { iteration }, AgentEvent::ConfirmationRequired { .. }) => {
                Ok(AgentState::AwaitingConfirmation {
                    iteration: *iteration,
                })
            }
            (AgentState::AwaitingConfirmation { iteration }, AgentEvent::TextDelta { .. }) => {
                Ok(AgentState::Thinking {
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { .. }, AgentEvent::Error { error, recoverable }) => {
                Ok(AgentState::Failed {
                    error: error.clone(),
                    recoverable: *recoverable,
                })
            }
            (AgentState::Planning { .. }, AgentEvent::Error { error, recoverable }) => {
                Ok(AgentState::Failed {
                    error: error.clone(),
                    recoverable: *recoverable,
                })
            }
            (AgentState::ToolExecution { .. }, AgentEvent::Error { error, recoverable }) => {
                Ok(AgentState::Failed {
                    error: error.clone(),
                    recoverable: *recoverable,
                })
            }
            (AgentState::Planning { iteration, .. }, AgentEvent::SummarizationStarted) => {
                Ok(AgentState::Summarizing {
                    iteration: *iteration,
                })
            }
            (AgentState::Thinking { iteration }, AgentEvent::SummarizationStarted) => {
                Ok(AgentState::Summarizing {
                    iteration: *iteration,
                })
            }
            (AgentState::Summarizing { .. }, AgentEvent::ContextSummarized { .. }) => self
                .post_summary_resume
                .take()
                .ok_or_else(|| AgentError::SessionFailed {
                    message: "missing post_summary_resume for ContextSummarized".into(),
                }),
            _ => Err(AgentError::InvalidTransition {
                from: self.state.to_string(),
                to: event.to_string(),
            }),
        }
    }

    fn accumulate_usage_cost(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
    ) {
        if self.config.max_budget_usd.is_none() {
            return;
        }
        let free = self.provider.name().eq_ignore_ascii_case("ollama");
        let openrouter = self.provider.name() == "OpenRouter";
        let est = estimate_cost_usd_with_rows(
            u64::from(input_tokens),
            u64::from(output_tokens),
            u64::from(cache_read_tokens),
            self.provider.completion_model_id(),
            openrouter,
            free,
            &self.config.model_estimates,
        );
        if let Some(d) = est {
            self.total_cost_usd += d;
            if let Some(max) = self.config.max_budget_usd
                && self.total_cost_usd >= max
            {
                self.budget_stop_before_next_iteration = true;
            }
        }
    }

    fn record_tool_completion_metrics(&mut self, result: &ToolCallResult) {
        self.reliability_metrics.tool_calls_total =
            self.reliability_metrics.tool_calls_total.saturating_add(1);
        if result.success {
            self.reliability_metrics.tool_calls_success = self
                .reliability_metrics
                .tool_calls_success
                .saturating_add(1);
        } else {
            self.reliability_metrics.tool_calls_failure = self
                .reliability_metrics
                .tool_calls_failure
                .saturating_add(1);
        }
        self.reliability_metrics.tool_latency_ms_total = self
            .reliability_metrics
            .tool_latency_ms_total
            .saturating_add(result.latency_ms);
        self.tool_latency_samples_ms.push(result.latency_ms);

        let total = self.reliability_metrics.tool_calls_total;
        self.reliability_metrics.tool_latency_ms_avg = if total == 0 {
            0
        } else {
            self.reliability_metrics.tool_latency_ms_total / total
        };
        self.reliability_metrics.tool_latency_ms_p95 = None;

        if matches!(&result.output, ToolOutput::Error { .. })
            && tool_output_is_timeout(&result.output)
        {
            self.reliability_metrics.timeouts_total =
                self.reliability_metrics.timeouts_total.saturating_add(1);
        }
    }

    fn record_policy_denial(&mut self) {
        self.reliability_metrics.policy_denials_total = self
            .reliability_metrics
            .policy_denials_total
            .saturating_add(1);
    }

    fn record_retry_attempt(&mut self) {
        self.reliability_metrics.retries_total =
            self.reliability_metrics.retries_total.saturating_add(1);
    }

    fn record_timeout_if_model_error(&mut self, err: &ModelError) {
        if model_error_is_timeout(err) {
            self.reliability_metrics.timeouts_total =
                self.reliability_metrics.timeouts_total.saturating_add(1);
        }
    }

    fn reliability_metrics_snapshot(&self) -> RunReliabilityMetrics {
        let mut snapshot = self.reliability_metrics.clone();
        snapshot.tool_latency_ms_p95 = percentile_95(&self.tool_latency_samples_ms);
        snapshot
    }

    async fn apply_event(
        &mut self,
        tx: &mpsc::Sender<AgentEvent>,
        event: AgentEvent,
        task: &str,
    ) -> Result<(), AgentError> {
        let tool_done = match &event {
            AgentEvent::ToolCallCompleted {
                name,
                success,
                message,
                ..
            } => Some(ToolCallSummary {
                name: name.clone(),
                success: *success,
                message: message.clone(),
            }),
            _ => None,
        };

        validate_transition(&self.state, &event)?;
        if let AgentEvent::UsageReport {
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
        } = &event
        {
            self.last_usage = Some(UsageReport {
                input_tokens: *input_tokens,
                output_tokens: *output_tokens,
                cache_creation_tokens: *cache_creation_tokens,
                cache_read_tokens: *cache_read_tokens,
            });
            self.total_input_tokens = self.total_input_tokens.saturating_add(*input_tokens);
            self.total_cache_read_tokens = self
                .total_cache_read_tokens
                .saturating_add(*cache_read_tokens);
            self.total_output_tokens = self.total_output_tokens.saturating_add(*output_tokens);
            self.accumulate_usage_cost(*input_tokens, *output_tokens, *cache_read_tokens);
        }
        self.state = self.next_state_after(&event, task)?;
        self.audit_log.push(AuditEvent::AgentStep {
            session_id: self.config.session_id.to_string(),
            timestamp: Utc::now(),
            description: event.to_string(),
        });
        match &event {
            AgentEvent::ToolCallDispatched {
                id,
                name: _,
                arguments: _,
            } => {
                if let Some(mcp) = self.mcp_call_context_by_id.get(id) {
                    self.audit_log.push(AuditEvent::ToolDispatch {
                        session_id: self.config.session_id.to_string(),
                        timestamp: Utc::now(),
                        tool_name: "mcp_tool".to_string(),
                        input_summary: "MCP call arguments redacted".to_string(),
                        mcp_server: Some(mcp.server.clone()),
                        mcp_tool: Some(mcp.tool.clone()),
                        decision_reason: Some(
                            "mcp dispatch allowed after policy evaluation".to_string(),
                        ),
                    });
                }
            }
            AgentEvent::ToolCallCompleted {
                id,
                success,
                message,
                ..
            } => {
                if let Some(mcp) = self.mcp_call_context_by_id.remove(id) {
                    self.audit_log.push(AuditEvent::ToolOutcome {
                        session_id: self.config.session_id.to_string(),
                        timestamp: Utc::now(),
                        tool_name: "mcp_tool".to_string(),
                        outcome: if *success {
                            ToolOutcomeKind::Success
                        } else {
                            ToolOutcomeKind::Failure
                        },
                        summary: message.clone(),
                        mcp_server: Some(mcp.server),
                        mcp_tool: Some(mcp.tool),
                        decision_reason: None,
                    });
                }
            }
            _ => {}
        }
        if let Some(t) = tool_done {
            self.tool_call_summaries.push(t);
        }
        tx.send(event)
            .await
            .map_err(|_| AgentError::SessionFailed {
                message: "agent event receiver dropped".into(),
            })?;
        Ok(())
    }
}

fn summarize_tool_output(output: &ToolOutput) -> (bool, String) {
    match output {
        ToolOutput::Success { content } => (true, content.clone()),
        ToolOutput::Error { message, .. } => (false, message.clone()),
        ToolOutput::Question { question, .. } => (false, question.clone()),
    }
}

fn tool_output_is_timeout(output: &ToolOutput) -> bool {
    if let ToolOutput::Error { message, .. } = output {
        let lower = message.to_ascii_lowercase();
        return lower.contains("timed out") || lower.contains("timeout");
    }
    false
}

fn model_error_is_timeout(error: &ModelError) -> bool {
    match error {
        ModelError::FirstTokenTimeout => true,
        ModelError::StreamInterrupted { message } => {
            let lower = message.to_ascii_lowercase();
            lower.contains("timed out") || lower.contains("timeout")
        }
        _ => false,
    }
}

fn percentile_95(samples: &[u64]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let rank = (n.saturating_mul(95).saturating_add(99)) / 100;
    let idx = rank.saturating_sub(1).min(n.saturating_sub(1));
    sorted.get(idx).copied()
}

fn summarize_policy_decisions(audit: &[AuditEvent]) -> PolicyDecisionSummary {
    let mut allow = 0_u64;
    let mut deny = 0_u64;
    let mut prompted = 0_u64;
    let mut samples = Vec::new();
    for event in audit {
        if let AuditEvent::PolicyEvaluation {
            permission,
            verdict,
            reason,
            ..
        } = event
        {
            match verdict {
                PolicyVerdict::Allow => allow = allow.saturating_add(1),
                PolicyVerdict::Deny => deny = deny.saturating_add(1),
            }
            let lowered = reason.to_ascii_lowercase();
            if lowered.contains("interactive")
                || lowered.contains("user approved")
                || lowered.contains("user denied")
            {
                prompted = prompted.saturating_add(1);
            }
            if samples.len() < 20 {
                let permission_kind = match permission {
                    Permission::ReadFile { .. } => "read_file",
                    Permission::ListDirectory { .. } => "list_directory",
                    Permission::WriteFile { .. } => "write_file",
                    Permission::ExecuteCommand { .. } => "execute_command",
                    Permission::NetworkFetch { .. } => "network_fetch",
                };
                let verdict_text = match verdict {
                    PolicyVerdict::Allow => "allow",
                    PolicyVerdict::Deny => "deny",
                };
                samples.push(format!("{verdict_text}:{permission_kind}:{reason}"));
            }
        }
    }
    PolicyDecisionSummary {
        allow,
        deny,
        prompted,
        decision_samples: samples,
    }
}

/// Runs one tool after policy approval (no events; used for parallel [`Tool::execute`]).
pub async fn execute_single_tool_call(
    call: &PendingToolCall,
    tool: &dyn Tool,
    ctx: &ToolContext,
) -> ToolCallResult {
    let arguments = call.arguments.clone();
    let started = Instant::now();
    let output = tool.execute(call.arguments.clone(), ctx).await;
    let elapsed_ms_u128 = started.elapsed().as_millis();
    let latency_ms = u64::try_from(elapsed_ms_u128).unwrap_or(u64::MAX);
    let (success, _) = summarize_tool_output(&output);
    ToolCallResult {
        call_id: call.id.clone(),
        tool_name: call.name.clone(),
        output,
        success,
        arguments,
        latency_ms,
    }
}

fn map_model_error(e: ModelError) -> AgentError {
    AgentError::ModelError {
        message: e.to_string(),
    }
}

fn occurrences_of(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

async fn file_change_diff_preview(
    sandbox: &Sandbox,
    tool_name: &str,
    args: &Value,
) -> Option<String> {
    match tool_name {
        "write_spec" => {
            let name = args.get("name").and_then(|v| v.as_str())?;
            let new_c = args.get("content").and_then(|v| v.as_str())?;
            let path = akmon_tools::relative_markdown_path_for_spec_name(name)?;
            let full = sandbox.resolve(&path).ok()?;
            let old = tokio::fs::read_to_string(&full).await.unwrap_or_default();
            Some(unified_diff_text(&old, new_c, &path))
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str())?;
            let new_c = args.get("content").and_then(|v| v.as_str())?;
            let full = sandbox.resolve(path).ok()?;
            let old = tokio::fs::read_to_string(&full).await.unwrap_or_default();
            Some(unified_diff_text(&old, new_c, path))
        }
        "edit" => {
            let path = args.get("path").and_then(|v| v.as_str())?;
            let old_str = args.get("old_str").and_then(|v| v.as_str())?;
            let new_str = args.get("new_str").and_then(|v| v.as_str())?;
            let full = sandbox.resolve(path).ok()?;
            let bytes = tokio::fs::read(&full).await.ok()?;
            let content = String::from_utf8(bytes).ok()?;
            if occurrences_of(&content, old_str) != 1 {
                return None;
            }
            let new_content = content.replacen(old_str, new_str, 1);
            Some(unified_diff_text(&content, &new_content, path))
        }
        _ => None,
    }
}

fn git_concrete_permissions(args: &Value) -> Vec<Permission> {
    let sub = args
        .get("subcommand")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let argv: Vec<String> = args
        .get("args")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let repo = PathBuf::from(".");
    match sub {
        "status" | "diff" | "log" | "show" => vec![Permission::ReadFile { path: repo }],
        "stash" => {
            if argv.first().map(|s| s.as_str()) == Some("list") {
                vec![Permission::ReadFile { path: repo }]
            } else {
                vec![Permission::WriteFile { path: repo }]
            }
        }
        "branch" => {
            if argv.is_empty() {
                vec![Permission::ReadFile { path: repo }]
            } else {
                vec![Permission::WriteFile { path: repo }]
            }
        }
        "add" | "commit" | "restore" => vec![Permission::WriteFile { path: repo }],
        _ => vec![Permission::ReadFile { path: repo }],
    }
}

/// Builds concrete [`Permission`] values for policy checks (paths from args when known).
fn concrete_permissions(
    tool: &dyn Tool,
    name: &str,
    args: &Value,
    sandbox_root: &Path,
) -> Vec<Permission> {
    match name {
        "read_file" => {
            if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                vec![Permission::ReadFile {
                    path: PathBuf::from(p),
                }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "write_file" => {
            if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                vec![Permission::WriteFile {
                    path: PathBuf::from(p),
                }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "edit" => {
            if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                vec![Permission::WriteFile {
                    path: PathBuf::from(p),
                }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "patch" => {
            if let Some(p) = args.get("patch").and_then(|v| v.as_str()) {
                match akmon_tools::patch_write_relative_paths(p) {
                    Some(paths) => paths
                        .into_iter()
                        .map(|path| Permission::WriteFile { path })
                        .collect(),
                    None => tool.required_permissions().to_vec(),
                }
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "apply_patch" => {
            if let Some(p) = args.get("file_path").and_then(|v| v.as_str()) {
                vec![Permission::WriteFile {
                    path: PathBuf::from(p),
                }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "list_directory" => {
            if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                vec![Permission::ListDirectory {
                    path: PathBuf::from(p),
                }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "shell" => {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                vec![Permission::ExecuteCommand {
                    command: cmd.trim().to_string(),
                    cwd: sandbox_root.to_path_buf(),
                }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "search" => {
            let p = args
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(".");
            vec![Permission::ReadFile {
                path: PathBuf::from(p),
            }]
        }
        "web_fetch" => {
            if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
                vec![Permission::NetworkFetch { url: u.to_string() }]
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "semantic_search" => {
            vec![Permission::ReadFile {
                path: PathBuf::from("."),
            }]
        }
        "write_spec" => {
            if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
                if let Some(rel) = akmon_tools::relative_markdown_path_for_spec_name(name) {
                    vec![Permission::WriteFile {
                        path: PathBuf::from(rel),
                    }]
                } else {
                    tool.required_permissions().to_vec()
                }
            } else {
                tool.required_permissions().to_vec()
            }
        }
        "read_spec" => {
            if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
                let t = name.trim().trim_end_matches(".md");
                if !t.is_empty() && !t.contains('/') && !t.contains('\\') {
                    vec![Permission::ReadFile {
                        path: PathBuf::from(format!(".akmon/specs/{t}.md")),
                    }]
                } else {
                    tool.required_permissions().to_vec()
                }
            } else {
                vec![Permission::ListDirectory {
                    path: PathBuf::from(".akmon/specs"),
                }]
            }
        }
        "spawn_subagent" => vec![],
        "git" => git_concrete_permissions(args),
        _ => tool.required_permissions().to_vec(),
    }
}

impl<S, G> Drop for AgentSession<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    fn drop(&mut self) {
        if !self.journal_started.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.try_emit_session_end_once(None) {
            tracing::warn!(
                target: "akmon::session",
                session_id = %self.config.session_id,
                error = %e,
                "journal SessionEnd emission failed on drop"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use akmon_core::{
        FilesystemPolicyConfig, PatternRuleSet, PolicyConfig, PolicyEngine, PolicyEngineMode,
        ToolPolicyConfig,
    };
    use akmon_journal::{
        AttemptStatus, EventKind, Hash, HashAlgorithm, JournalError, MemoryObjectStore,
        MemorySessionGraph, ObjectStore, SessionGraph, VerificationReport,
    };
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use uuid::Uuid;

    use crate::journal::JournalHandle;

    fn test_sandbox(dir: &std::path::Path) -> Arc<Sandbox> {
        Arc::new(Sandbox::new(dir))
    }

    fn test_journal_sid(session_id: Uuid) -> JournalHandle<MemoryObjectStore, MemorySessionGraph> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
            Arc::clone(&store),
            session_id,
        )));
        JournalHandle::new(store, graph)
    }

    fn test_journal() -> JournalHandle<MemoryObjectStore, MemorySessionGraph> {
        test_journal_sid(Uuid::nil())
    }

    /// [`SessionGraph`] that rejects [`EventKind::UserTurn`] after [`MemorySessionGraph`] accepted `SessionStart`.
    struct RejectUserTurnAppend {
        inner: MemorySessionGraph,
    }

    impl SessionGraph for RejectUserTurnAppend {
        fn session_id(&self) -> Uuid {
            self.inner.session_id()
        }

        fn append(&mut self, kind: EventKind) -> akmon_journal::Result<Hash> {
            if matches!(kind, EventKind::UserTurn { .. }) {
                return Err(JournalError::Verification(
                    "test reject UserTurn append".into(),
                ));
            }
            self.inner.append(kind)
        }

        fn head(&self) -> akmon_journal::Result<Option<Hash>> {
            self.inner.head()
        }

        fn history(&self) -> akmon_journal::Result<Vec<(Hash, akmon_journal::Event)>> {
            self.inner.history()
        }

        fn verify(&self) -> akmon_journal::Result<VerificationReport> {
            self.inner.verify()
        }

        fn import_verified_linear_history(
            &mut self,
            events: &[(Hash, akmon_journal::Event)],
        ) -> akmon_journal::Result<()> {
            self.inner.import_verified_linear_history(events)
        }
    }

    fn test_journal_reject_user_turn_append(
        session_id: Uuid,
    ) -> JournalHandle<MemoryObjectStore, RejectUserTurnAppend> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let inner = MemorySessionGraph::open_new(Arc::clone(&store), session_id);
        let graph = Arc::new(Mutex::new(RejectUserTurnAppend { inner }));
        JournalHandle::new(store, graph)
    }

    /// [`SessionGraph`] that rejects [`EventKind::PermissionGate`] only (other events delegate).
    struct RejectPermissionGateAppend {
        inner: MemorySessionGraph,
    }

    impl SessionGraph for RejectPermissionGateAppend {
        fn session_id(&self) -> Uuid {
            self.inner.session_id()
        }

        fn append(&mut self, kind: EventKind) -> akmon_journal::Result<Hash> {
            if matches!(kind, EventKind::PermissionGate { .. }) {
                return Err(JournalError::Verification(
                    "test reject PermissionGate append".into(),
                ));
            }
            self.inner.append(kind)
        }

        fn head(&self) -> akmon_journal::Result<Option<Hash>> {
            self.inner.head()
        }

        fn history(&self) -> akmon_journal::Result<Vec<(Hash, akmon_journal::Event)>> {
            self.inner.history()
        }

        fn verify(&self) -> akmon_journal::Result<VerificationReport> {
            self.inner.verify()
        }

        fn import_verified_linear_history(
            &mut self,
            events: &[(Hash, akmon_journal::Event)],
        ) -> akmon_journal::Result<()> {
            self.inner.import_verified_linear_history(events)
        }
    }

    fn test_journal_reject_permission_gate(
        session_id: Uuid,
    ) -> JournalHandle<MemoryObjectStore, RejectPermissionGateAppend> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let inner = MemorySessionGraph::open_new(Arc::clone(&store), session_id);
        let graph = Arc::new(Mutex::new(RejectPermissionGateAppend { inner }));
        JournalHandle::new(store, graph)
    }

    /// [`SessionGraph`] that rejects [`EventKind::AssistantTurn`] only.
    struct RejectAssistantTurnAppend {
        inner: MemorySessionGraph,
    }

    impl SessionGraph for RejectAssistantTurnAppend {
        fn session_id(&self) -> Uuid {
            self.inner.session_id()
        }

        fn append(&mut self, kind: EventKind) -> akmon_journal::Result<Hash> {
            if matches!(kind, EventKind::AssistantTurn { .. }) {
                return Err(JournalError::Verification(
                    "test reject AssistantTurn append".into(),
                ));
            }
            self.inner.append(kind)
        }

        fn head(&self) -> akmon_journal::Result<Option<Hash>> {
            self.inner.head()
        }

        fn history(&self) -> akmon_journal::Result<Vec<(Hash, akmon_journal::Event)>> {
            self.inner.history()
        }

        fn verify(&self) -> akmon_journal::Result<VerificationReport> {
            self.inner.verify()
        }

        fn import_verified_linear_history(
            &mut self,
            events: &[(Hash, akmon_journal::Event)],
        ) -> akmon_journal::Result<()> {
            self.inner.import_verified_linear_history(events)
        }
    }

    fn test_journal_reject_assistant_turn(
        session_id: Uuid,
    ) -> JournalHandle<MemoryObjectStore, RejectAssistantTurnAppend> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let inner = MemorySessionGraph::open_new(Arc::clone(&store), session_id);
        let graph = Arc::new(Mutex::new(RejectAssistantTurnAppend { inner }));
        JournalHandle::new(store, graph)
    }

    fn journal_event_kind_tags(h: &[(Hash, akmon_journal::Event)]) -> Vec<&'static str> {
        h.iter()
            .map(|(_, e)| match &e.kind {
                EventKind::SessionStart { .. } => "SessionStart",
                EventKind::UserTurn { .. } => "UserTurn",
                EventKind::ProviderCall { .. } => "ProviderCall",
                EventKind::ToolCall { .. } => "ToolCall",
                EventKind::PermissionGate { .. } => "PermissionGate",
                EventKind::AssistantTurn { .. } => "AssistantTurn",
                EventKind::RetrievalCall { .. } => "RetrievalCall",
                EventKind::SessionEnd { .. } => "SessionEnd",
            })
            .collect()
    }

    fn last_permission_gate_before_tool_call(
        h: &[(Hash, akmon_journal::Event)],
    ) -> Option<(&str, &str)> {
        let tc_i = h
            .iter()
            .position(|(_, e)| matches!(e.kind, EventKind::ToolCall { .. }))?;
        let mut out = None;
        for (_, e) in h.iter().take(tc_i) {
            if let EventKind::PermissionGate {
                policy_id,
                decision,
                ..
            } = &e.kind
            {
                out = Some((policy_id.as_str(), decision.as_str()));
            }
        }
        out
    }

    #[test]
    fn session_starts_idle() {
        let tmp = tempfile::tempdir().expect("tmp");
        let s = AgentSession::new(
            AgentConfig {
                max_iterations: 25,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            test_journal(),
        )
        .unwrap();
        assert!(matches!(s.state(), AgentState::Idle));
    }

    #[test]
    fn iteration_limit_second_attempt_errors() {
        let config = AgentConfig {
            max_iterations: 1,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
            auto_commit: false,
            max_completion_tokens: None,
            subagent_style: false,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
        };
        assert!(check_iteration_limit(0, &config).is_ok());
        assert_eq!(
            check_iteration_limit(1, &config),
            Err(AgentError::IterationLimitReached { limit: 1 })
        );
    }

    struct StubProvider {
        events: Vec<Result<StreamEvent, ModelError>>,
    }

    impl StubProvider {
        fn empty_end_turn() -> Self {
            Self {
                events: vec![Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                })],
            }
        }
    }

    #[async_trait]
    impl LlmProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn context_window_tokens(&self) -> usize {
            200_000
        }

        fn completion_model_id(&self) -> &str {
            "stub-model"
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _config: &CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            let v = self.events.clone();
            Ok(Box::pin(stream::iter(v)))
        }
    }

    /// Drives [`ContextManager::needs_summarization`] true while delegating completions to [`StubProvider`].
    struct HighEstimateProvider(StubProvider);

    impl HighEstimateProvider {
        fn for_summarization_test() -> Self {
            Self(StubProvider::empty_end_turn())
        }
    }

    #[async_trait]
    impl LlmProvider for HighEstimateProvider {
        fn name(&self) -> &str {
            "ollama"
        }

        fn context_window_tokens(&self) -> usize {
            2_500
        }

        fn completion_model_id(&self) -> &str {
            self.0.completion_model_id()
        }

        fn estimate_tokens(&self, _messages: &[Message]) -> Option<usize> {
            Some(50_000)
        }

        async fn complete(
            &self,
            messages: &[Message],
            config: &CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            self.0.complete(messages, config).await
        }
    }

    /// Returns a fresh stream per [`LlmProvider::complete`] call from `sequences[call_index]`.
    struct SeqProvider {
        sequences: Vec<Vec<Result<StreamEvent, ModelError>>>,
        call: AtomicUsize,
    }

    impl SeqProvider {
        fn new(sequences: Vec<Vec<Result<StreamEvent, ModelError>>>) -> Self {
            Self {
                sequences,
                call: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for SeqProvider {
        fn name(&self) -> &str {
            "seq"
        }

        fn context_window_tokens(&self) -> usize {
            200_000
        }

        fn completion_model_id(&self) -> &str {
            "seq-model"
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _config: &CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            let i = self.call.fetch_add(1, Ordering::SeqCst);
            let events = self.sequences.get(i).cloned().unwrap_or_default();
            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[tokio::test]
    async fn max_tokens_then_end_turn_continues_without_error() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![
            vec![
                Ok(StreamEvent::TextDelta {
                    text: "partial".into(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::MaxTokens,
                    tool_calls: vec![],
                }),
            ],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        let r = session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await;
        assert!(r.is_ok(), "{r:?}");
        assert!(
            session.result_text().contains("partial"),
            "result_text={}",
            session.result_text()
        );

        let mut saw_trunc_status = false;
        while let Ok(e) = rx.try_recv() {
            if let AgentEvent::StatusInfo { message } = e
                && message.contains("truncated, continuing")
                && message.contains("(1/3)")
            {
                saw_trunc_status = true;
            }
        }
        assert!(saw_trunc_status, "expected continuation StatusInfo");
    }

    #[tokio::test]
    async fn max_tokens_with_tool_calls_errors() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::MaxTokens,
            tool_calls: vec![ModelToolCall {
                id: "t".into(),
                name: "read_file".into(),
                arguments: json!({}),
            }],
        })]]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let err = session
            .run("t".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect_err("expected model error");
        assert!(matches!(err, AgentError::ModelError { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn multi_turn_unknown_tool_then_end_turn_emits_completed_and_done() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "c1".into(),
                    name: "unknown_x".into(),
                    arguments: json!({}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        let r = session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await;
        assert!(r.is_ok());

        let mut names = Vec::new();
        while let Ok(e) = rx.try_recv() {
            match e {
                AgentEvent::ToolCallDispatched { name, .. } => names.push(format!("disp:{name}")),
                AgentEvent::ToolCallCompleted { name, success, .. } => {
                    names.push(format!("done:{name}:{success}"))
                }
                AgentEvent::Done => names.push("Done".into()),
                _ => {}
            }
        }
        assert!(
            !names.iter().any(|s| s.starts_with("disp:")),
            "unknown tool must not emit ToolCallDispatched, got {names:?}"
        );
        assert!(
            names.iter().any(|s| s.contains("unknown_x")),
            "expected unknown tool completion, got {names:?}"
        );
        assert!(
            names.iter().any(|s| s == "Done"),
            "expected Done, got {names:?}"
        );
    }

    #[tokio::test]
    async fn iteration_limit_on_repeated_tool_use() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let tool_round = vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
            tool_calls: vec![ModelToolCall {
                id: "c1".into(),
                name: "missing".into(),
                arguments: json!({}),
            }],
        })];
        let seq = SeqProvider::new(vec![tool_round.clone(), tool_round.clone(), tool_round]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 2,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let err = session
            .run("t".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect_err("expected iteration error");
        assert!(
            matches!(err, AgentError::IterationLimitReached { limit: 2 }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn unknown_tool_yields_completed_failure() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "x".into(),
                    name: "nope".into(),
                    arguments: json!({}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("hi".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let mut saw = false;
        while let Ok(e) = rx.try_recv() {
            if let AgentEvent::ToolCallCompleted {
                success: false,
                message,
                name,
                ..
            } = e
                && name == "nope"
                && message.contains("tool not found")
            {
                saw = true;
            }
        }
        assert!(saw, "expected failed ToolCallCompleted for unknown tool");
    }

    /// Test-only tool that sleeps then returns a marker (for parallel timing / ordering tests).
    struct DelayTool {
        id: &'static str,
        ms: u64,
    }

    fn delay_tool_perms() -> &'static [Permission] {
        static CELL: std::sync::OnceLock<Vec<Permission>> = std::sync::OnceLock::new();
        CELL.get_or_init(|| {
            vec![Permission::ReadFile {
                path: PathBuf::from("."),
            }]
        })
        .as_slice()
    }

    #[async_trait]
    impl Tool for DelayTool {
        fn name(&self) -> &str {
            self.id
        }

        fn description(&self) -> &str {
            "delay test tool"
        }

        fn required_permissions(&self) -> &[Permission] {
            delay_tool_perms()
        }

        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
            tokio::time::sleep(std::time::Duration::from_millis(self.ms)).await;
            ToolOutput::Success {
                content: format!("done-{}", self.id),
            }
        }
    }

    /// Minimal tool for journal `ToolCall` integration tests (fixed success body).
    struct JournalEmitTool {
        id: &'static str,
    }

    #[async_trait]
    impl Tool for JournalEmitTool {
        fn name(&self) -> &str {
            self.id
        }

        fn description(&self) -> &str {
            "journal emit test tool"
        }

        fn required_permissions(&self) -> &[Permission] {
            delay_tool_perms()
        }

        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::Success {
                content: "journal-fixed".into(),
            }
        }
    }

    struct TimeoutTool;

    #[async_trait]
    impl Tool for TimeoutTool {
        fn name(&self) -> &str {
            "timeout_tool"
        }

        fn description(&self) -> &str {
            "returns timeout-like error"
        }

        fn required_permissions(&self) -> &[Permission] {
            delay_tool_perms()
        }

        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::Error {
                code: akmon_tools::ToolErrorCode::SubprocessFailed,
                message: "command timed out after 30s".into(),
            }
        }
    }

    struct TestMcpTool {
        call_count: Arc<AtomicUsize>,
        server: String,
        tool: String,
    }

    fn test_mcp_perms() -> &'static [Permission] {
        static CELL: std::sync::OnceLock<Vec<Permission>> = std::sync::OnceLock::new();
        CELL.get_or_init(|| {
            vec![Permission::NetworkFetch {
                url: "https://mcp.example.test".into(),
            }]
        })
        .as_slice()
    }

    #[async_trait]
    impl Tool for TestMcpTool {
        fn name(&self) -> &str {
            "mcp_tool"
        }

        fn description(&self) -> &str {
            "test MCP tool"
        }

        fn required_permissions(&self) -> &[Permission] {
            test_mcp_perms()
        }

        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> ToolOutput {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            ToolOutput::Success {
                content: "mcp-ok".into(),
            }
        }

        fn mcp_policy_context(&self) -> Option<akmon_tools::McpPolicyContext> {
            Some(akmon_tools::McpPolicyContext {
                server: self.server.clone(),
                tool: self.tool.clone(),
            })
        }
    }

    #[tokio::test]
    async fn two_parallel_reads_context_order_matches_request_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let p1 = dir.path().join("a.txt");
        let p2 = dir.path().join("b.txt");
        tokio::fs::write(&p1, b"alpha").await.expect("w");
        tokio::fs::write(&p2, b"beta").await.expect("w");

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "a.txt"}),
                    },
                    ModelToolCall {
                        id: "2".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "b.txt"}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 2);
        assert!(tool_msgs[0].content.contains("a.txt") || tool_msgs[0].content.contains("alpha"));
        assert!(tool_msgs[1].content.contains("b.txt") || tool_msgs[1].content.contains("beta"));
        assert!(tool_msgs[0].content.contains("\"tool_call_id\":\"1\""));
        assert!(tool_msgs[1].content.contains("\"tool_call_id\":\"2\""));
    }

    #[tokio::test]
    async fn session_accumulates_usage_totals_across_model_rounds() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        tokio::fs::write(dir.path().join("f.txt"), b"x")
            .await
            .expect("w");

        let seq = SeqProvider::new(vec![
            vec![
                Ok(StreamEvent::UsageReport(UsageReport {
                    input_tokens: 100,
                    output_tokens: 5,
                    cache_creation_tokens: 80,
                    cache_read_tokens: 0,
                })),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    tool_calls: vec![ModelToolCall {
                        id: "1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "f.txt"}),
                    }],
                }),
            ],
            vec![
                Ok(StreamEvent::UsageReport(UsageReport {
                    input_tokens: 200,
                    output_tokens: 12,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 150,
                })),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                }),
            ],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        assert_eq!(session.total_input_tokens(), 300);
        assert_eq!(session.total_cache_read_tokens(), 150);
        assert_eq!(session.total_output_tokens(), 17);
        let last = session.last_usage().expect("last usage");
        assert_eq!(last.input_tokens, 200);
        assert_eq!(last.cache_read_tokens, 150);
    }

    #[tokio::test]
    async fn parallel_delay_tools_complete_in_wall_time_not_sum() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "a".into(),
                        name: "d100a".into(),
                        arguments: json!({}),
                    },
                    ModelToolCall {
                        id: "b".into(),
                        name: "d100b".into(),
                        arguments: json!({}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![
                Box::new(DelayTool {
                    id: "d100a",
                    ms: 100,
                }),
                Box::new(DelayTool {
                    id: "d100b",
                    ms: 100,
                }),
            ],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let t0 = Instant::now();
        session
            .run("x".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 180,
            "expected ~100ms parallel wall time, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn policy_denies_one_parallel_call_other_still_executes() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let p = dir.path().join("r.txt");
        tokio::fs::write(&p, b"ok").await.expect("w");

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "r1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "r.txt"}),
                    },
                    ModelToolCall {
                        id: "w1".into(),
                        name: "write_file".into(),
                        arguments: json!({"path": "x.txt", "content": "n"}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: false,
            })),
            Arc::new(seq),
            vec![
                Box::new(akmon_tools::ReadFileTool::new()),
                Box::new(akmon_tools::WriteFileTool::new()),
            ],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 2);
        assert!(
            tool_msgs[0].content.contains("ok"),
            "first tool should be successful read, got {}",
            tool_msgs[0].content
        );
        assert!(
            tool_msgs[1].content.contains("policy denied")
                || tool_msgs[1].content.contains("PermissionDenied"),
            "second should be write denial, got {}",
            tool_msgs[1].content
        );
    }

    #[tokio::test]
    async fn reliability_metrics_track_tool_success_failure_policy_denial_and_latency() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let p = dir.path().join("r.txt");
        tokio::fs::write(&p, b"ok").await.expect("w");
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "r1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "r.txt"}),
                    },
                    ModelToolCall {
                        id: "w1".into(),
                        name: "write_file".into(),
                        arguments: json!({"path": "x.txt", "content": "n"}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: false,
            })),
            Arc::new(seq),
            vec![
                Box::new(akmon_tools::ReadFileTool::new()),
                Box::new(akmon_tools::WriteFileTool::new()),
            ],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let m = session.reliability_metrics();
        assert_eq!(m.tool_calls_total, 2);
        assert_eq!(m.tool_calls_success, 1);
        assert_eq!(m.tool_calls_failure, 1);
        assert_eq!(m.policy_denials_total, 1);
        assert!(m.tool_latency_ms_total >= m.tool_latency_ms_avg);
        assert!(m.tool_latency_ms_p95.is_some());
        let evidence = session.evidence_data();
        assert_eq!(evidence.reliability_metrics.tool_calls_total, 2);
    }

    fn mcp_single_call_provider() -> SeqProvider {
        SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "m1".into(),
                    name: "mcp_tool".into(),
                    arguments: json!({"q":"x"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ])
    }

    fn default_test_agent_config() -> AgentConfig {
        AgentConfig {
            max_iterations: 5,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
            auto_commit: false,
            max_completion_tokens: None,
            subagent_style: false,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
        }
    }

    #[tokio::test]
    async fn mcp_server_denied_blocks_call() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let cfg = PolicyConfig {
            mcp: akmon_core::McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec!["github".into()],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec![],
                },
            },
            network: akmon_core::NetworkPolicyConfig {
                allow_domains: vec!["mcp.example.test".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let mut session = AgentSession::new(
            default_test_agent_config(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(mcp_single_call_provider()),
            vec![Box::new(TestMcpTool {
                call_count: Arc::clone(&counter),
                server: "github".into(),
                tool: "search_issues".into(),
            })],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn mcp_tool_denied_while_server_allowed_blocks_call() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let cfg = PolicyConfig {
            mcp: akmon_core::McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec![],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec!["search_issues".into()],
                },
            },
            network: akmon_core::NetworkPolicyConfig {
                allow_domains: vec!["mcp.example.test".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let mut session = AgentSession::new(
            default_test_agent_config(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(mcp_single_call_provider()),
            vec![Box::new(TestMcpTool {
                call_count: Arc::clone(&counter),
                server: "github".into(),
                tool: "search_issues".into(),
            })],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn mcp_allowed_server_and_tool_executes_and_audit_is_enriched() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let cfg = PolicyConfig {
            mcp: akmon_core::McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec![],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec![],
                },
            },
            network: akmon_core::NetworkPolicyConfig {
                allow_domains: vec!["mcp.example.test".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let mut session = AgentSession::new(
            default_test_agent_config(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(mcp_single_call_provider()),
            vec![Box::new(TestMcpTool {
                call_count: Arc::clone(&counter),
                server: "github".into(),
                tool: "search_issues".into(),
            })],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        let found = session.audit_events().iter().any(|e| match e {
            AuditEvent::PolicyEvaluation {
                mcp_server,
                mcp_tool,
                decision_reason,
                ..
            } => {
                mcp_server.as_deref() == Some("github")
                    && mcp_tool.as_deref() == Some("search_issues")
                    && decision_reason.is_some()
            }
            _ => false,
        });
        assert!(found, "expected MCP-enriched policy evaluation audit event");
        let found_outcome = session.audit_events().iter().any(|e| match e {
            AuditEvent::ToolOutcome {
                mcp_server,
                mcp_tool,
                ..
            } => {
                mcp_server.as_deref() == Some("github")
                    && mcp_tool.as_deref() == Some("search_issues")
            }
            _ => false,
        });
        assert!(
            found_outcome,
            "expected MCP-enriched tool outcome audit event"
        );
    }

    #[tokio::test]
    async fn mcp_missing_context_fails_closed() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let cfg = PolicyConfig {
            mcp: akmon_core::McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["github".into()],
                    deny: vec![],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec![],
                },
            },
            network: akmon_core::NetworkPolicyConfig {
                allow_domains: vec!["mcp.example.test".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let mut session = AgentSession::new(
            default_test_agent_config(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(mcp_single_call_provider()),
            vec![Box::new(TestMcpTool {
                call_count: Arc::clone(&counter),
                server: String::new(),
                tool: "search_issues".into(),
            })],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn mcp_ambiguous_context_fails_closed_without_execution() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let cfg = PolicyConfig {
            mcp: akmon_core::McpPolicyConfig {
                servers: PatternRuleSet {
                    allow: vec!["*".into()],
                    deny: vec![],
                },
                tools: PatternRuleSet {
                    allow: vec!["search_*".into()],
                    deny: vec![],
                },
            },
            network: akmon_core::NetworkPolicyConfig {
                allow_domains: vec!["mcp.example.test".into()],
                deny_domains: vec![],
            },
            ..PolicyConfig::default()
        };
        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        let mut session = AgentSession::new(
            default_test_agent_config(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(mcp_single_call_provider()),
            vec![
                Box::new(TestMcpTool {
                    call_count: Arc::clone(&c1),
                    server: "github".into(),
                    tool: "search_issues".into(),
                }),
                Box::new(TestMcpTool {
                    call_count: Arc::clone(&c2),
                    server: "jira".into(),
                    tool: "search_issues".into(),
                }),
            ],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(c1.load(Ordering::SeqCst), 0);
        assert_eq!(c2.load(Ordering::SeqCst), 0);
        let denied = session
            .context_messages()
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .any(|m| m.content.contains("ambiguous MCP context"));
        assert!(
            denied,
            "expected explicit ambiguous MCP denial in tool output"
        );
    }

    #[tokio::test]
    async fn mcp_has_no_bypass_in_autoapprove_reads_and_fetch_mode() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let counter = Arc::new(AtomicUsize::new(0));
        let mut session = AgentSession::new(
            default_test_agent_config(),
            Arc::new(PolicyEngine::new(
                PolicyEngineMode::AutoApproveReadsAndFetch {
                    confirm_writes: true,
                },
            )),
            Arc::new(mcp_single_call_provider()),
            vec![Box::new(TestMcpTool {
                call_count: Arc::clone(&counter),
                server: "github".into(),
                tool: "search_issues".into(),
            })],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "MCP must not bypass fail-closed governance path"
        );
    }

    #[tokio::test]
    async fn reliability_metrics_count_retry_continuations() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::MaxTokens,
                tool_calls: vec![],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(session.reliability_metrics().retries_total, 1);
    }

    #[tokio::test]
    async fn reliability_metrics_count_tool_timeout_failures() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "t1".into(),
                    name: "timeout_tool".into(),
                    arguments: json!({}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(TimeoutTool)],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let m = session.reliability_metrics();
        assert_eq!(m.tool_calls_failure, 1);
        assert_eq!(m.timeouts_total, 1);
    }

    #[tokio::test]
    async fn configured_tool_policy_denies_dispatch_even_when_permission_would_allow() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let p = dir.path().join("r.txt");
        tokio::fs::write(&p, b"ok").await.expect("w");

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "r1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "r.txt"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let cfg = PolicyConfig {
            tools: ToolPolicyConfig {
                allow: vec!["read_*".into()],
                deny: vec!["read_file".into()],
            },
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["r.txt".into()],
                    deny: vec![],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert!(
            tool_msgs[0].content.contains("policy denied")
                || tool_msgs[0].content.contains("PermissionDenied"),
            "tool should be denied by tool policy, got {}",
            tool_msgs[0].content
        );
    }

    #[tokio::test]
    async fn configured_tool_policy_allow_and_permission_allow_executes_tool() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let p = dir.path().join("r.txt");
        tokio::fs::write(&p, b"ok").await.expect("w");

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "r1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "r.txt"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let cfg = PolicyConfig {
            tools: ToolPolicyConfig {
                allow: vec!["read_*".into()],
                deny: vec![],
            },
            filesystem: FilesystemPolicyConfig {
                read: PatternRuleSet {
                    allow: vec!["r.txt".into()],
                    deny: vec![],
                },
                write: PatternRuleSet::default(),
            },
            ..PolicyConfig::default()
        };

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Configured(cfg))),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert!(
            tool_msgs[0].content.contains("ok"),
            "tool should be allowed and return file contents, got {}",
            tool_msgs[0].content
        );
    }

    #[tokio::test]
    async fn tool_results_appended_in_request_order_not_completion_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "slow".into(),
                        name: "slow_z".into(),
                        arguments: json!({}),
                    },
                    ModelToolCall {
                        id: "fast".into(),
                        name: "fast_a".into(),
                        arguments: json!({}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![
                Box::new(DelayTool {
                    id: "slow_z",
                    ms: 120,
                }),
                Box::new(DelayTool {
                    id: "fast_a",
                    ms: 15,
                }),
            ],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 2);
        assert!(
            tool_msgs[0].content.contains("done-slow_z"),
            "first slot must be slow tool (request order), got {}",
            tool_msgs[0].content
        );
        assert!(
            tool_msgs[1].content.contains("done-fast_a"),
            "second slot must be fast tool, got {}",
            tool_msgs[1].content
        );
    }

    #[tokio::test]
    async fn third_write_file_in_same_assistant_message_is_rejected() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "w1".into(),
                        name: "write_file".into(),
                        arguments: json!({"path": "a.txt", "content": "a"}),
                    },
                    ModelToolCall {
                        id: "w2".into(),
                        name: "write_file".into(),
                        arguments: json!({"path": "b.txt", "content": "b"}),
                    },
                    ModelToolCall {
                        id: "w3".into(),
                        name: "write_file".into(),
                        arguments: json!({"path": "c.txt", "content": "c"}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(8);
        policy_tx
            .send(InteractivePolicyReply::allow_once())
            .await
            .expect("policy sender");
        policy_tx
            .send(InteractivePolicyReply::allow_once())
            .await
            .expect("policy sender");

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::WriteFileTool::new())],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut policy_opt = Some(policy_rx);
        session
            .run("x".into(), ev_tx, &mut policy_opt, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 3, "one tool result per call");
        let third: serde_json::Value = serde_json::from_str(&tool_msgs[2].content).expect("json");
        assert_eq!(third["output"]["status"], "error");
        assert!(
            third["output"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("Only two write_file")),
            "expected cap message, got {:?}",
            third["output"]["message"]
        );
        assert!(dir.path().join("a.txt").is_file());
        assert!(dir.path().join("b.txt").is_file());
        assert!(!dir.path().join("c.txt").exists());
    }

    #[tokio::test]
    async fn tool_success_appends_tool_role_message() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("f.txt");
        tokio::fs::write(&p, b"hello").await.expect("write");
        let sandbox = test_sandbox(dir.path());

        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "r1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "f.txt"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);

        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(4);
        let _ = policy_tx.send(InteractivePolicyReply::allow_once()).await;

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (ev_tx, _ev_rx) = mpsc::channel(64);
        let mut policy_opt = Some(policy_rx);
        session
            .run("read".into(), ev_tx, &mut policy_opt, &mut None, None)
            .await
            .expect("run");

        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert!(tool_msgs[0].content.contains("hello"));
        assert!(tool_msgs[0].content.contains("read_file"));
    }

    #[tokio::test]
    async fn result_text_accumulates_text_deltas() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![vec![
            Ok(StreamEvent::TextDelta { text: "hel".into() }),
            Ok(StreamEvent::TextDelta { text: "lo".into() }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ]]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        assert_eq!(session.result_text(), "hello");
    }

    #[tokio::test]
    async fn audit_events_roundtrip_as_jsonl_after_run() {
        use akmon_core::write_audit_jsonl;

        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
        })]]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let audit_file = dir.path().join("audit.jsonl");
        write_audit_jsonl(&audit_file, session.audit_events()).expect("write audit");

        let contents = std::fs::read_to_string(&audit_file).expect("read audit");
        assert!(!contents.trim().is_empty());
        for line in contents.lines() {
            let parsed: akmon_core::AuditChainRecord =
                serde_json::from_str(line).expect("valid AuditChainRecord JSONL line");
            assert!(parsed.event.to_json().is_ok());
        }
    }

    #[tokio::test]
    async fn replay_metadata_present_and_does_not_expose_prompt_secret() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
        })]]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();

        let secret = "sk-test-secret-in-user-task";
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run(secret.to_string(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");

        let replay = session.replay_metadata().expect("replay metadata");
        let serialized = serde_json::to_string(replay).expect("serialize");
        assert!(!serialized.contains(secret));
        assert!(replay.prompt_assembly_hash.is_some());
    }

    #[tokio::test]
    async fn evidence_data_sorts_and_dedups_files() {
        let dir = tempfile::tempdir().expect("tmp");
        let sandbox = test_sandbox(dir.path());
        let seq = SeqProvider::new(vec![vec![Ok(StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
        })]]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
            test_journal(),
        )
        .unwrap();
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        session.modified_paths = vec![
            PathBuf::from("b.rs"),
            PathBuf::from("a.rs"),
            PathBuf::from("a.rs"),
        ];
        let evidence = session.evidence_data();
        assert_eq!(
            evidence.files_touched,
            vec!["a.rs".to_string(), "b.rs".to_string()]
        );
        assert!(evidence.replay_metadata.is_some());
    }

    #[test]
    fn concrete_permissions_write_spec_matches_sanitized_file() {
        let tool = akmon_tools::WriteSpecTool::new();
        let root = std::path::Path::new("/tmp");
        let p = concrete_permissions(
            &tool,
            "write_spec",
            &json!({ "name": "My Spec", "content": "x" }),
            root,
        );
        assert_eq!(
            p,
            vec![Permission::WriteFile {
                path: PathBuf::from(".akmon/specs/My-Spec.md"),
            }]
        );
    }

    #[test]
    fn concrete_permissions_write_spec_unsafe_name_falls_back() {
        let tool = akmon_tools::WriteSpecTool::new();
        let root = std::path::Path::new("/tmp");
        let p = concrete_permissions(
            &tool,
            "write_spec",
            &json!({ "name": "../../etc", "content": "x" }),
            root,
        );
        assert_eq!(p, tool.required_permissions().to_vec());
    }

    #[test]
    fn should_write_handoff_requires_min_turns_and_signal() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            test_journal(),
        )
        .unwrap();
        assert!(!crate::specs_and_handoff::should_write_handoff(&s));
        s.user_turns_finished = crate::specs_and_handoff::MIN_USER_TURNS_FOR_HANDOFF;
        assert!(!crate::specs_and_handoff::should_write_handoff(&s));
        s.last_assistant_snippet = Some("done".into());
        assert!(crate::specs_and_handoff::should_write_handoff(&s));
    }

    #[test]
    fn write_handoff_file_writes_when_eligible() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            test_journal(),
        )
        .unwrap();
        s.user_turns_finished = crate::specs_and_handoff::MIN_USER_TURNS_FOR_HANDOFF;
        s.last_assistant_snippet = Some("summary".into());
        crate::specs_and_handoff::write_handoff_file(&s, tmp.path(), "test-model")
            .expect("handoff");
        let body = std::fs::read_to_string(crate::specs_and_handoff::handoff_path(tmp.path()))
            .expect("read");
        assert!(body.contains("test-model"));
        assert!(body.contains("summary"));
    }

    #[test]
    fn journal_session_start_emitted_once_at_new() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let s = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let h = s.journal_history_snapshot().expect("history");
        assert_eq!(h.len(), 1);
        assert!(matches!(h[0].1.kind, EventKind::SessionStart { .. }));
        let head = {
            let guard = s
                .journal
                .graph
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.head().expect("head query")
        };
        assert_eq!(head, Some(h[0].0.clone()));
    }

    fn config_hash_from_session_start(
        s: &AgentSession<MemoryObjectStore, MemorySessionGraph>,
    ) -> Hash {
        let h = s.journal_history_snapshot().expect("history");
        match &h[0].1.kind {
            EventKind::SessionStart { config_hash, .. } => config_hash.clone(),
            k => panic!("expected SessionStart, got {k:?}"),
        }
    }

    #[test]
    fn t_session_config_hash_deterministic() {
        let sid = Uuid::new_v4();
        let cfg = AgentConfig {
            max_iterations: 11,
            confirmation_timeout_secs: 45,
            session_id: sid,
            auto_commit: true,
            max_completion_tokens: Some(4096),
            subagent_style: true,
            max_budget_usd: Some(2.25),
            fallback_model: Some("fallback-model-id".into()),
            model_estimates: Vec::new(),
        };
        let tmp = tempfile::tempdir().expect("tmp");
        let j1 = test_journal_sid(sid);
        let s1 = AgentSession::new(
            cfg.clone(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j1,
        )
        .expect("s1");
        let c1 = config_hash_from_session_start(&s1);
        drop(s1);

        let j2 = test_journal_sid(sid);
        let s2 = AgentSession::new(
            cfg.clone(),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j2,
        )
        .expect("s2");
        let c2 = config_hash_from_session_start(&s2);
        assert_eq!(c1, c2);
    }

    #[tokio::test]
    async fn t_user_turn_emitted_after_prepare() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let task = "known user turn text";
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run(task.into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        assert_eq!(h.len(), 4, "{h:?}");
        assert!(matches!(h[0].1.kind, EventKind::SessionStart { .. }));
        let prompt_hash = match &h[1].1.kind {
            EventKind::UserTurn { prompt_hash } => prompt_hash.clone(),
            ref k => panic!("expected UserTurn, got {k:?}"),
        };
        assert!(
            matches!(h[2].1.kind, EventKind::ProviderCall { .. }),
            "expected ProviderCall, got {:?}",
            h[2].1.kind
        );
        assert!(
            matches!(h[3].1.kind, EventKind::AssistantTurn { .. }),
            "expected AssistantTurn, got {:?}",
            h[3].1.kind
        );
        let bytes = session
            .journal
            .store
            .get(&prompt_hash)
            .expect("get")
            .expect("blob");
        assert_eq!(bytes.as_ref(), task.as_bytes());
    }

    #[tokio::test]
    async fn t_user_turn_emission_failure_returns_err() {
        let sid = Uuid::new_v4();
        let j = test_journal_reject_user_turn_append(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        assert_eq!(session.journal_history_snapshot().expect("h0").len(), 1);
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let err = session
            .run("any task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect_err("run should fail when UserTurn append fails");
        let msg = err.to_string();
        assert!(
            msg.contains("test reject UserTurn append"),
            "unexpected err: {msg}"
        );
        assert_eq!(session.journal_history_snapshot().expect("h1").len(), 1);
    }

    #[tokio::test]
    async fn t_multiple_run_calls_emit_multiple_user_turns() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("first".into(), tx.clone(), &mut no_policy, &mut None, None)
            .await
            .expect("run1");
        session
            .run("second".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run2");
        let h = session.journal_history_snapshot().expect("hist");
        assert_eq!(h.len(), 7, "{h:?}");
        assert!(matches!(h[0].1.kind, EventKind::SessionStart { .. }));
        let ut: Vec<_> = h
            .iter()
            .filter_map(|(_, e)| match &e.kind {
                EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ut.len(), 2);
        let b1 = session
            .journal
            .store
            .get(&ut[0])
            .expect("g1")
            .expect("blob1");
        let b2 = session
            .journal
            .store
            .get(&ut[1])
            .expect("g2")
            .expect("blob2");
        assert_eq!(b1.as_ref(), b"first");
        assert_eq!(b2.as_ref(), b"second");
        assert_ne!(ut[0], ut[1]);
    }

    #[tokio::test]
    async fn t_provider_call_event_emitted_during_run() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let stub = StubProvider::empty_end_turn();
        let expected_id = akmon_models::canonical_provider_id(&stub);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(stub),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("hello".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let pc = h.iter().find_map(|(_, e)| match &e.kind {
            EventKind::ProviderCall {
                provider_id,
                attempts,
                ..
            } => Some((provider_id.clone(), attempts.clone())),
            _ => None,
        });
        let (pid, attempts) = pc.expect("ProviderCall");
        assert_eq!(pid, expected_id);
        assert!(!attempts.is_empty(), "{attempts:?}");
    }

    #[tokio::test]
    async fn t_provider_call_attempts_captured_for_mock() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let attempts = h
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::ProviderCall { attempts, .. } => Some(attempts.clone()),
                _ => None,
            })
            .expect("ProviderCall");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt_number, 1);
        assert_eq!(attempts[0].status, AttemptStatus::Success);
    }

    #[tokio::test]
    async fn t_summarization_path_emits_provider_call() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(HighEstimateProvider::for_summarization_test()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let bulk: Vec<Message> = (0..16)
            .map(|i| Message {
                role: MessageRole::User,
                content: format!("bulk-{i}-{}", "y".repeat(500)),
            })
            .collect();
        session.restore_context_from_messages(bulk);
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task line".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let n_pc = h
            .iter()
            .filter(|(_, e)| matches!(e.kind, EventKind::ProviderCall { .. }))
            .count();
        assert!(
            n_pc >= 2,
            "expected summarization + main ProviderCall, got {h:?}"
        );
    }

    fn journal_test_canonical_cbor<T: serde::Serialize + ?Sized>(v: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(v, &mut bytes).expect("cbor");
        bytes
    }

    #[tokio::test]
    async fn t_tool_call_event_emitted_during_dispatch() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let tool_name = "journal_emit_tool";
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "tc1".into(),
                    name: tool_name.into(),
                    arguments: json!({"n": 7}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(JournalEmitTool { id: tool_name })],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let first_pc = h
            .iter()
            .position(|(_, e)| matches!(e.kind, EventKind::ProviderCall { .. }))
            .expect("ProviderCall");
        let tc_idx = h
            .iter()
            .enumerate()
            .find_map(|(i, (_, e))| matches!(e.kind, EventKind::ToolCall { .. }).then_some(i))
            .expect("ToolCall");
        assert!(
            tc_idx > first_pc,
            "ToolCall should follow first ProviderCall: {h:?}"
        );
        match &h[tc_idx].1.kind {
            EventKind::ToolCall { tool_id, .. } => assert_eq!(tool_id, tool_name),
            k => panic!("expected ToolCall, got {k:?}"),
        }
    }

    #[tokio::test]
    async fn t_tool_call_input_output_resolvable_in_store() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let tool_name = "journal_emit_tool";
        let args = json!({"n": 7});
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "tc1".into(),
                    name: tool_name.into(),
                    arguments: args.clone(),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(JournalEmitTool { id: tool_name })],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let (input_hash, output_hash, side_fx) = h
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::ToolCall {
                    input_hash,
                    output_hash,
                    side_effects_hash,
                    ..
                } => Some((
                    input_hash.clone(),
                    output_hash.clone(),
                    side_effects_hash.clone(),
                )),
                _ => None,
            })
            .expect("ToolCall");
        assert!(
            side_fx.is_none(),
            "expected no side_effects_hash, got {side_fx:?}"
        );
        let expected_in = journal_test_canonical_cbor(&args);
        let expected_out = journal_test_canonical_cbor(&ToolOutput::Success {
            content: "journal-fixed".into(),
        });
        let got_in = session
            .journal
            .store
            .get(&input_hash)
            .expect("get in")
            .expect("blob in");
        let got_out = session
            .journal
            .store
            .get(&output_hash)
            .expect("get out")
            .expect("blob out");
        assert_eq!(got_in.as_ref(), expected_in.as_slice());
        assert_eq!(got_out.as_ref(), expected_out.as_slice());
    }

    #[tokio::test]
    async fn t_multiple_tools_each_emit_tool_call() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![
                    ModelToolCall {
                        id: "a".into(),
                        name: "journal_emit_a".into(),
                        arguments: json!({}),
                    },
                    ModelToolCall {
                        id: "b".into(),
                        name: "journal_emit_b".into(),
                        arguments: json!({}),
                    },
                ],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![
                Box::new(JournalEmitTool {
                    id: "journal_emit_a",
                }),
                Box::new(JournalEmitTool {
                    id: "journal_emit_b",
                }),
            ],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let ids: Vec<String> = h
            .iter()
            .filter_map(|(_, e)| match &e.kind {
                EventKind::ToolCall { tool_id, .. } => Some(tool_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2, "expected two ToolCall events, got {ids:?}");
        assert_ne!(ids[0], ids[1]);
        assert!(ids.contains(&"journal_emit_a".into()));
        assert!(ids.contains(&"journal_emit_b".into()));
    }

    #[tokio::test]
    async fn t_permission_gate_emitted_for_automatic_allow() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let tool_name = "journal_emit_tool";
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "g1".into(),
                    name: tool_name.into(),
                    arguments: json!({}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(JournalEmitTool { id: tool_name })],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let (pid, dec) = last_permission_gate_before_tool_call(&h).expect("pg before tc");
        assert_eq!(pid, "tool:auto");
        assert_eq!(dec, "allowed");
    }

    #[tokio::test]
    async fn t_permission_gate_emitted_for_denial() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let tool_name = "journal_emit_tool";
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "g1".into(),
                    name: tool_name.into(),
                    arguments: json!({}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![Box::new(JournalEmitTool { id: tool_name })],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        assert!(
            !h.iter()
                .any(|(_, e)| matches!(e.kind, EventKind::ToolCall { .. })),
            "denied tool must not execute: {h:?}"
        );
        let pg = h.iter().find_map(|(_, e)| match &e.kind {
            EventKind::PermissionGate {
                policy_id,
                decision,
                ..
            } => Some((policy_id.as_str(), decision.as_str())),
            _ => None,
        });
        let (pid, dec) = pg.expect("PermissionGate");
        assert_eq!(pid, "tool:auto");
        assert_eq!(dec, "denied");
    }

    #[tokio::test]
    async fn t_permission_gate_emitted_for_interactive_confirmation() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "w1".into(),
                    name: "write_file".into(),
                    arguments: json!({"path": "out.txt", "content": "x"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(4);
        let _ = policy_tx.send(InteractivePolicyReply::allow_once()).await;

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::WriteFileTool::new())],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut policy_opt = Some(policy_rx);
        session
            .run("task".into(), tx, &mut policy_opt, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let (pid, dec) = last_permission_gate_before_tool_call(&h).expect("pg before tc");
        assert_eq!(pid, "interactive");
        assert_eq!(dec, "allowed");
        assert!(tmp.path().join("out.txt").is_file());
    }

    #[tokio::test]
    async fn t_permission_gate_emitted_for_remembered_approval() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "w1".into(),
                    name: "write_file".into(),
                    arguments: json!({"path": "mem.txt", "content": "y"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::WriteFileTool::new())],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        session.test_set_permission_allow_all_writes(true);
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let (pid, dec) = last_permission_gate_before_tool_call(&h).expect("pg before tc");
        assert_eq!(pid, "remembered:write");
        assert_eq!(dec, "allowed");
        assert!(tmp.path().join("mem.txt").is_file());
    }

    #[tokio::test]
    async fn t_permission_gate_failure_does_not_break_policy() {
        let sid = Uuid::new_v4();
        let j = test_journal_reject_permission_gate(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        tokio::fs::write(tmp.path().join("f.txt"), b"z")
            .await
            .expect("w");
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: vec![ModelToolCall {
                    id: "r1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "f.txt"}),
                }],
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("go".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let n_pg = h
            .iter()
            .filter(|(_, e)| matches!(e.kind, EventKind::PermissionGate { .. }))
            .count();
        assert_eq!(n_pg, 0, "PermissionGate append was rejected");
        assert!(
            h.iter()
                .any(|(_, e)| matches!(e.kind, EventKind::ToolCall { .. })),
            "tool should still run: {h:?}"
        );
        let tool_msgs: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert!(tool_msgs[0].content.contains('z'));
    }

    #[tokio::test]
    async fn t_assistant_turn_emitted_for_pure_text_response() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let seq = SeqProvider::new(vec![vec![
            Ok(StreamEvent::TextDelta {
                text: "hola".into(),
            }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ]]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let tc_none = h.iter().find_map(|(_, e)| match &e.kind {
            EventKind::AssistantTurn {
                tool_calls_hash, ..
            } => Some(tool_calls_hash.is_none()),
            _ => None,
        });
        assert_eq!(
            tc_none,
            Some(true),
            "expected AssistantTurn with tool_calls_hash None: {h:?}"
        );
    }

    #[tokio::test]
    async fn t_assistant_turn_emitted_with_tool_calls() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let tool_name = "journal_emit_tool";
        let model_calls = vec![ModelToolCall {
            id: "c1".into(),
            name: tool_name.into(),
            arguments: json!({"k": 1}),
        }];
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: model_calls.clone(),
            })],
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            })],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(JournalEmitTool { id: tool_name })],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("go".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let (msg_h, tc_h) = h
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::AssistantTurn {
                    message_hash,
                    tool_calls_hash,
                } => tool_calls_hash
                    .as_ref()
                    .map(|tch| (message_hash.clone(), tch.clone())),
                _ => None,
            })
            .expect("AssistantTurn with tool_calls");
        let expected_tc = crate::journal::assistant_tool_calls_cbor(&model_calls).expect("cbor");
        let got_tc = session
            .journal
            .store
            .get(&tc_h)
            .expect("get tc")
            .expect("blob tc");
        assert_eq!(got_tc.as_ref(), expected_tc.as_slice());
        let got_msg = session
            .journal
            .store
            .get(&msg_h)
            .expect("get msg")
            .expect("blob msg");
        assert_eq!(got_msg.as_ref(), b"");
    }

    #[tokio::test]
    async fn t_assistant_turn_message_hash_resolves_to_content() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let body = "plain_body_verify";
        let seq = SeqProvider::new(vec![vec![
            Ok(StreamEvent::TextDelta { text: body.into() }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ]]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let msg_h = h
            .iter()
            .find_map(|(_, e)| match &e.kind {
                EventKind::AssistantTurn { message_hash, .. } => Some(message_hash.clone()),
                _ => None,
            })
            .expect("AssistantTurn");
        let got = session
            .journal
            .store
            .get(&msg_h)
            .expect("get")
            .expect("blob");
        assert_eq!(got.as_ref(), body.as_bytes());
    }

    #[tokio::test]
    async fn t_assistant_turn_failure_does_not_break_session() {
        let sid = Uuid::new_v4();
        let j = test_journal_reject_assistant_turn(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let body = "survives_journal_loss";
        let seq = SeqProvider::new(vec![vec![
            Ok(StreamEvent::TextDelta { text: body.into() }),
            Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
            }),
        ]]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        assert!(
            !h.iter()
                .any(|(_, e)| matches!(e.kind, EventKind::AssistantTurn { .. })),
            "AssistantTurn append rejected: {h:?}"
        );
        let mut saw_delta = false;
        while let Ok(e) = rx.try_recv() {
            if let AgentEvent::TextDelta { text } = e
                && text == body
            {
                saw_delta = true;
            }
        }
        assert!(saw_delta, "expected streamed TextDelta");
        let assistants: Vec<_> = session
            .context
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .collect();
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].content, body);
    }

    #[tokio::test]
    async fn t_full_event_sequence_for_simple_turn() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let tool_name = "journal_emit_tool";
        let model_calls = vec![ModelToolCall {
            id: "z9".into(),
            name: tool_name.into(),
            arguments: json!({}),
        }];
        let seq = SeqProvider::new(vec![
            vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                tool_calls: model_calls.clone(),
            })],
            vec![
                Ok(StreamEvent::TextDelta { text: "fin".into() }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                }),
            ],
        ]);
        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(JournalEmitTool { id: tool_name })],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("one".into(), tx, &mut no_policy, &mut None, None)
            .await
            .expect("run");
        let h = session.journal_history_snapshot().expect("hist");
        let tags = journal_event_kind_tags(&h);
        let expected = vec![
            "SessionStart",
            "UserTurn",
            "ProviderCall",
            "PermissionGate",
            "ToolCall",
            "AssistantTurn",
            "ProviderCall",
            "AssistantTurn",
        ];
        assert_eq!(tags, expected, "unexpected sequence: {h:?}");
    }

    #[test]
    fn t_session_end_via_explicit_end_emits_once() {
        let sid = Uuid::new_v4();
        let j = test_journal_sid(sid);
        let tmp = tempfile::tempdir().expect("tmp");
        let s = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: sid,
                auto_commit: false,
                max_completion_tokens: None,
                subagent_style: false,
                max_budget_usd: None,
                fallback_model: None,
                model_estimates: Vec::new(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
            j,
        )
        .expect("session");
        assert_eq!(s.journal_history_snapshot().expect("h").len(), 1);
        s.end(None).expect("end");
        assert_eq!(s.journal_history_snapshot().expect("h2").len(), 2);
        s.end(None).expect("end idempotent");
        assert_eq!(s.journal_history_snapshot().expect("h3").len(), 2);
        assert!(matches!(
            s.journal_history_snapshot().expect("h4")[1].1.kind,
            EventKind::SessionEnd { .. }
        ));
    }

    #[test]
    fn t_session_end_explicit_then_drop_does_not_double_emit() {
        let sid = Uuid::new_v4();
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
            Arc::clone(&store),
            sid,
        )));
        let j = JournalHandle::new(Arc::clone(&store), Arc::clone(&graph));
        let tmp = tempfile::tempdir().expect("tmp");
        {
            let s = AgentSession::new(
                AgentConfig {
                    max_iterations: 5,
                    confirmation_timeout_secs: 30,
                    session_id: sid,
                    auto_commit: false,
                    max_completion_tokens: None,
                    subagent_style: false,
                    max_budget_usd: None,
                    fallback_model: None,
                    model_estimates: Vec::new(),
                },
                Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
                Arc::new(StubProvider::empty_end_turn()),
                vec![],
                test_sandbox(tmp.path()),
                None,
                false,
                j,
            )
            .expect("session");
            assert_eq!(s.journal_history_snapshot().expect("h").len(), 1);
            s.end(None).expect("end");
            assert_eq!(s.journal_history_snapshot().expect("h2").len(), 2);
        }
        let guard = graph.lock().unwrap_or_else(|p| p.into_inner());
        let h = guard.history().expect("h3");
        assert_eq!(h.len(), 2);
        assert!(matches!(h[1].1.kind, EventKind::SessionEnd { .. }));
    }

    #[test]
    fn t_session_end_via_drop_emits_once() {
        let sid = Uuid::new_v4();
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
            Arc::clone(&store),
            sid,
        )));
        let j = JournalHandle::new(Arc::clone(&store), Arc::clone(&graph));
        let tmp = tempfile::tempdir().expect("tmp");
        {
            let _s = AgentSession::new(
                AgentConfig {
                    max_iterations: 5,
                    confirmation_timeout_secs: 30,
                    session_id: sid,
                    auto_commit: false,
                    max_completion_tokens: None,
                    subagent_style: false,
                    max_budget_usd: None,
                    fallback_model: None,
                    model_estimates: Vec::new(),
                },
                Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
                Arc::new(StubProvider::empty_end_turn()),
                vec![],
                test_sandbox(tmp.path()),
                None,
                false,
                j,
            )
            .expect("session");
        }
        let guard = graph.lock().unwrap_or_else(|p| p.into_inner());
        let h = guard.history().expect("h");
        assert_eq!(h.len(), 2);
        assert!(matches!(h[1].1.kind, EventKind::SessionEnd { .. }));
    }
}
