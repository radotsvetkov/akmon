//! Agent session: owns FSM state, provider, tools, and the main query loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use akmon_core::{
    AgentConfig, AgentError, AgentEvent, AgentState, AuditEvent, InteractivePolicyReply,
    Permission, PolicyEngineError, PolicyEngineMode, PolicyVerdict, Sandbox, check_iteration_limit,
    validate_transition,
};
use akmon_models::{
    CompletionConfig, CompletionStream, LlmProvider, Message, MessageRole, ModelError,
    ModelToolCall, StopReason, StreamEvent, ToolDefinition, UsageReport,
    anthropic_system_block_text, approximate_tokens, max_tokens_for_model,
};
use akmon_tools::{Tool, ToolContext, ToolOutput, unified_diff_text};
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::context::{build_followup_messages, build_messages};
use crate::context_manager::ContextManager;

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
}

/// Builds [`CompletionConfig`] with `tools` populated and model-appropriate [`CompletionConfig::max_tokens`].
fn completion_config_for_tools(
    tools: &[Arc<dyn Tool>],
    provider: &dyn LlmProvider,
) -> CompletionConfig {
    let defs: Vec<ToolDefinition> = tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.as_ref().name().to_string(),
            description: t.as_ref().description().to_string(),
            parameters: t.as_ref().parameters_schema(),
        })
        .collect();
    CompletionConfig {
        tools: defs,
        max_tokens: max_tokens_for_model(provider.completion_model_id()),
        ..CompletionConfig::default()
    }
}

/// Owns one running agent: configuration, FSM state, policy, model backend, tool registry, chat
/// history, audit trail, optional `AKMON.md` text, and the filesystem [`Sandbox`].
pub struct AgentSession {
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
    /// When `true`, project system prompts are read-only (plan mode); tools should match.
    plan_mode: bool,
    /// Permissions the user allowed with “remember for session”; matched exactly before interactive prompts.
    permission_session_allowlist: Vec<Permission>,
    /// How many automatic max-token continuations have run this turn (reset on EndTurn, ToolUse, new user turn).
    pub continuation_count: u32,
}

impl AgentSession {
    /// Creates a session in [`AgentState::Idle`] with the given dependencies.
    pub fn new(
        config: AgentConfig,
        policy: Arc<akmon_core::PolicyEngine>,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        sandbox: Arc<Sandbox>,
        akmon_md: Option<String>,
        plan_mode: bool,
    ) -> Self {
        let max_tokens = provider.context_window_tokens().clamp(1, 100_000);
        let fixed_system_messages = if akmon_md.is_some() { 2 } else { 1 };
        let context_manager = ContextManager {
            max_tokens,
            threshold: ContextManager::default().threshold,
            keep_recent: ContextManager::default().keep_recent,
            fixed_system_messages,
        };
        let tools: Vec<Arc<dyn Tool>> = tools.into_iter().map(Arc::from).collect();
        Self {
            config,
            state: AgentState::Idle,
            policy,
            provider,
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
            plan_mode,
            permission_session_allowlist: Vec::new(),
            continuation_count: 0,
        }
    }

    /// Returns whether this session uses read-only plan-mode system prompts.
    pub fn plan_mode(&self) -> bool {
        self.plan_mode
    }

    /// Swaps the tool registry (e.g. between plan-only and full implementation turns).
    pub fn replace_tools(&mut self, tools: Vec<Box<dyn Tool>>) {
        self.tools = tools.into_iter().map(Arc::from).collect();
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
    pub async fn run(
        &mut self,
        task: String,
        event_tx: mpsc::Sender<AgentEvent>,
        interactive_policy_rx: &mut Option<mpsc::Receiver<InteractivePolicyReply>>,
        interrupt_after_current_tools: Option<Arc<AtomicBool>>,
    ) -> Result<(), AgentError> {
        self.prepare_for_new_user_turn()?;

        if !matches!(self.state, AgentState::Idle) {
            return Err(AgentError::SessionFailed {
                message: "AgentSession::run expected Idle state after prepare".into(),
            });
        }

        let mut iteration: u32 = 0;
        let mut user_line_committed = false;

        'session: loop {
            match &self.state {
                AgentState::Complete => return Ok(()),
                AgentState::Failed { error, .. } => return Err(error.clone()),
                _ => {}
            }

            check_iteration_limit(iteration, &self.config)?;

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
            let tool_name_strings: Vec<String> = self
                .tools
                .iter()
                .map(|t| t.as_ref().name().to_string())
                .collect();
            let tool_names: Vec<&str> = tool_name_strings.iter().map(|s| s.as_str()).collect();
            let messages = if user_line_committed {
                build_followup_messages(
                    self.akmon_md.as_deref(),
                    &self.context,
                    project_root.as_str(),
                    &tool_names,
                    self.plan_mode,
                )
            } else {
                build_messages(
                    self.akmon_md.as_deref(),
                    &self.context,
                    task.as_str(),
                    project_root.as_str(),
                    &tool_names,
                    self.plan_mode,
                )
            };

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
                messages = if user_line_committed {
                    build_followup_messages(
                        self.akmon_md.as_deref(),
                        &self.context,
                        project_root.as_str(),
                        &tool_names,
                        self.plan_mode,
                    )
                } else {
                    build_messages(
                        self.akmon_md.as_deref(),
                        &self.context,
                        task.as_str(),
                        project_root.as_str(),
                        &tool_names,
                        self.plan_mode,
                    )
                };
            }

            if std::env::var_os("AKMON_DEBUG_CACHE").as_deref() == Some(std::ffi::OsStr::new("1")) {
                let sys = anthropic_system_block_text(&messages);
                eprintln!(
                    "akmon: debug cache model_call={} system_joined_len={}",
                    iteration.saturating_add(1),
                    sys.len()
                );
            }

            let completion_config =
                completion_config_for_tools(&self.tools, self.provider.as_ref());
            let mut stream: CompletionStream =
                match self.provider.complete(&messages, &completion_config).await {
                    Ok(s) => s,
                    Err(e) => {
                        let ae = map_model_error(e);
                        self.apply_event(
                            &event_tx,
                            AgentEvent::Error {
                                error: ae.clone(),
                                recoverable: true,
                            },
                            &task,
                        )
                        .await?;
                        return Err(ae);
                    }
                };

            let mut accumulated = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Err(e) => {
                        let ae = map_model_error(e);
                        self.apply_event(
                            &event_tx,
                            AgentEvent::Error {
                                error: ae.clone(),
                                recoverable: true,
                            },
                            &task,
                        )
                        .await?;
                        return Err(ae);
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
                        let ae = map_model_error(error);
                        self.apply_event(
                            &event_tx,
                            AgentEvent::Error {
                                error: ae.clone(),
                                recoverable: true,
                            },
                            &task,
                        )
                        .await?;
                        return Err(ae);
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
                                        "text": accumulated,
                                        "tool_calls": tool_calls.iter().map(|c| {
                                            json!({"id": c.id, "name": c.name, "arguments": c.arguments})
                                        }).collect::<Vec<_>>(),
                                    });
                                    self.context.push(Message {
                                        role: MessageRole::Assistant,
                                        content: assistant_record.to_string(),
                                    });
                                    self.dispatch_tool_calls_batch(
                                        tool_calls,
                                        &event_tx,
                                        &task,
                                        interactive_policy_rx,
                                    )
                                    .await?;
                                    self.apply_event(
                                        &event_tx,
                                        AgentEvent::StatusInfo {
                                            message: "─ truncated mid-tool, resuming… ─".into(),
                                        },
                                        &task,
                                    )
                                    .await?;
                                    iteration = iteration.saturating_add(1);
                                    continue 'session;
                                }

                                if self.continuation_count < 3 {
                                    self.continuation_count =
                                        self.continuation_count.saturating_add(1);
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
                                        content: accumulated,
                                    });
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
                                        "text": accumulated,
                                        "tool_calls": tool_calls.iter().map(|c| {
                                            json!({"id": c.id, "name": c.name, "arguments": c.arguments})
                                        }).collect::<Vec<_>>(),
                                    });
                                    self.context.push(Message {
                                        role: MessageRole::Assistant,
                                        content: assistant_record.to_string(),
                                    });
                                    self.dispatch_tool_calls_batch(
                                        tool_calls,
                                        &event_tx,
                                        &task,
                                        interactive_policy_rx,
                                    )
                                    .await?;
                                    if interrupt_after_current_tools
                                        .as_ref()
                                        .is_some_and(|f| f.load(Ordering::SeqCst))
                                    {
                                        self.apply_event(&event_tx, AgentEvent::Done, &task).await?;
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
                                    content: accumulated,
                                });
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
                                    "text": accumulated,
                                    "tool_calls": tool_calls.iter().map(|c| {
                                        json!({"id": c.id, "name": c.name, "arguments": c.arguments})
                                    }).collect::<Vec<_>>(),
                                });
                                self.context.push(Message {
                                    role: MessageRole::Assistant,
                                    content: assistant_record.to_string(),
                                });

                                self.dispatch_tool_calls_batch(
                                    tool_calls,
                                    &event_tx,
                                    &task,
                                    interactive_policy_rx,
                                )
                                .await?;

                                if interrupt_after_current_tools
                                    .as_ref()
                                    .is_some_and(|f| f.load(Ordering::SeqCst))
                                {
                                    self.apply_event(&event_tx, AgentEvent::Done, &task).await?;
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

    /// Policy + optional parallel execution for one model turn's tool calls.
    async fn dispatch_tool_calls_batch(
        &mut self,
        tool_calls: Vec<ModelToolCall>,
        event_tx: &mpsc::Sender<AgentEvent>,
        task: &str,
        interactive_policy_rx: &mut Option<mpsc::Receiver<InteractivePolicyReply>>,
    ) -> Result<(), AgentError> {
        let n = tool_calls.len();
        let mut slots: Vec<Option<ToolCallResult>> = vec![None; n];

        struct ApprovedSlot {
            original_index: usize,
            id: String,
            name: String,
            arguments: Value,
            tool_idx: usize,
        }
        let mut approved: Vec<ApprovedSlot> = Vec::new();
        let mut write_file_calls_this_message: u32 = 0;

        for (idx, call) in tool_calls.iter().enumerate() {
            let id = call.id.clone();
            let name = call.name.clone();
            let args = call.arguments.clone();

            let Some(tool_idx) = self.tools.iter().position(|t| t.as_ref().name() == name) else {
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
                });
                continue;
            };

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
                    });
                    continue;
                }
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

            for perm in perms {
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
                    if !decision.allowed {
                        policy_denial_message = Some(format!("policy denied: {perm:?}"));
                        break;
                    }
                    continue;
                }

                let allowed = match self.policy.mode() {
                    PolicyEngineMode::Interactive
                    | PolicyEngineMode::AutoApproveReads { .. }
                    | PolicyEngineMode::AutoApproveReadsAndFetch { .. } => {
                        match self
                            .policy
                            .evaluate_automatic(session_id.as_str(), perm.clone())
                        {
                            Ok(decision) => {
                                self.audit_log.push(decision.audit.clone());
                                decision.allowed
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
                                if reply.remember_for_session
                                    && decision.allowed
                                    && !self.permission_session_allowlist.contains(&perm)
                                {
                                    self.permission_session_allowlist.push(perm.clone());
                                }
                                self.apply_event(
                                    event_tx,
                                    AgentEvent::TextDelta {
                                        text: String::new(),
                                    },
                                    task,
                                )
                                .await?;
                                decision.allowed
                            }
                            Err(e) => {
                                return Err(AgentError::SessionFailed {
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                    _ => {
                        let decision = match self
                            .policy
                            .evaluate_automatic(session_id.as_str(), perm.clone())
                        {
                            Ok(d) => d,
                            Err(e) => {
                                return Err(AgentError::SessionFailed {
                                    message: e.to_string(),
                                });
                            }
                        };
                        self.audit_log.push(decision.audit.clone());
                        decision.allowed
                    }
                };

                if !allowed {
                    policy_denial_message = Some(format!("policy denied: {perm:?}"));
                    break;
                }
            }

            if let Some(msg) = policy_denial_message {
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
                });
                continue;
            }

            approved.push(ApprovedSlot {
                original_index: idx,
                id,
                name,
                arguments: args,
                tool_idx,
            });
        }

        for a in &approved {
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
                    let ctx = ToolContext::new((*sandbox_c).clone(), policy_c);
                    let result = execute_single_tool_call(&pending, tool.as_ref(), &ctx).await;
                    (orig, result)
                });
            }

            while let Some((orig_idx, tool_result)) = unordered.next().await {
                let message = match &tool_result.output {
                    ToolOutput::Success { content } => content.clone(),
                    ToolOutput::Error { message, .. } => message.clone(),
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
            Err(_) => {
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
                Err(_) => {
                    self.restore_state_after_summarization_abort();
                    self.trim_oldest_non_system_fraction_of_context(0.2);
                    return Ok(());
                }
                Ok(StreamEvent::TextDelta { text }) => summary_text.push_str(&text),
                Ok(StreamEvent::UsageReport(_)) => {}
                Ok(StreamEvent::Error { .. }) => {
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
            (AgentState::Summarizing { iteration }, AgentEvent::StatusInfo { .. }) => {
                Ok(AgentState::Summarizing {
                    iteration: *iteration,
                })
            }
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
        }
        self.state = self.next_state_after(&event, task)?;
        self.audit_log.push(AuditEvent::AgentStep {
            session_id: self.config.session_id.to_string(),
            timestamp: Utc::now(),
            description: event.to_string(),
        });
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
    }
}

/// Runs one tool after policy approval (no events; used for parallel [`Tool::execute`]).
pub async fn execute_single_tool_call(
    call: &PendingToolCall,
    tool: &dyn Tool,
    ctx: &ToolContext,
) -> ToolCallResult {
    let arguments = call.arguments.clone();
    let output = tool.execute(call.arguments.clone(), ctx).await;
    let (success, _) = summarize_tool_output(&output);
    ToolCallResult {
        call_id: call.id.clone(),
        tool_name: call.name.clone(),
        output,
        success,
        arguments,
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
        "git" => git_concrete_permissions(args),
        _ => tool.required_permissions().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use akmon_core::{PolicyEngine, PolicyEngineMode};
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use uuid::Uuid;

    fn test_sandbox(dir: &std::path::Path) -> Arc<Sandbox> {
        Arc::new(Sandbox::new(dir))
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
            false,
        );
        assert!(matches!(s.state(), AgentState::Idle));
    }

    #[test]
    fn iteration_limit_second_attempt_errors() {
        let config = AgentConfig {
            max_iterations: 1,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
            auto_commit: false,
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        let r = session.run("task".into(), tx, &mut no_policy, None).await;
        assert!(r.is_ok(), "{r:?}");
        assert!(
            session.result_text().contains("partial"),
            "result_text={}",
            session.result_text()
        );

        let mut saw_trunc_status = false;
        while let Ok(e) = rx.try_recv() {
            if let AgentEvent::StatusInfo { message } = e {
                if message.contains("truncated, continuing") && message.contains("(1/3)") {
                    saw_trunc_status = true;
                }
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let err = session
            .run("t".into(), tx, &mut no_policy, None)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        let r = session.run("task".into(), tx, &mut no_policy, None).await;
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let err = session
            .run("t".into(), tx, &mut no_policy, None)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("hi".into(), tx, &mut no_policy, None)
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
            {
                if name == "nope" && message.contains("tool not found") {
                    saw = true;
                }
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
        );

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, None)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            })),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
        );

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("task".into(), ev_tx, &mut no_policy, None)
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
        );

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let t0 = Instant::now();
        session
            .run("x".into(), ev_tx, &mut no_policy, None)
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
        );

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, None)
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
        );

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("x".into(), ev_tx, &mut no_policy, None)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::WriteFileTool::new())],
            sandbox,
            None,
            false,
        );

        let (ev_tx, _rx) = mpsc::channel(64);
        let mut policy_opt = Some(policy_rx);
        session
            .run("x".into(), ev_tx, &mut policy_opt, None)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
            false,
        );

        let (ev_tx, _ev_rx) = mpsc::channel(64);
        let mut policy_opt = Some(policy_rx);
        session
            .run("read".into(), ev_tx, &mut policy_opt, None)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy, None)
            .await
            .expect("run");
        assert_eq!(session.result_text(), "hello");
    }

    #[tokio::test]
    async fn audit_events_roundtrip_as_jsonl_after_run() {
        use akmon_core::{AuditEvent, write_audit_jsonl};

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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
            false,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy, None)
            .await
            .expect("run");

        let audit_file = dir.path().join("audit.jsonl");
        write_audit_jsonl(&audit_file, session.audit_events()).expect("write audit");

        let contents = std::fs::read_to_string(&audit_file).expect("read audit");
        assert!(!contents.trim().is_empty());
        for line in contents.lines() {
            let parsed: AuditEvent =
                serde_json::from_str(line).expect("valid AuditEvent JSONL line");
            assert!(parsed.to_json().is_ok());
        }
    }
}
