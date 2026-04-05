//! Agent session: owns FSM state, provider, tools, and the main query loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use akmon_core::{
    check_iteration_limit, validate_transition, AgentConfig, AgentError, AgentEvent, AgentState,
    AuditEvent, Permission, PolicyEngineError, PolicyEngineMode, PolicyVerdict, Sandbox,
};
use akmon_models::{
    CompletionConfig, CompletionStream, LlmProvider, Message, MessageRole, ModelError,
    ModelToolCall, StopReason, StreamEvent, ToolDefinition,
};
use akmon_tools::{Tool, ToolContext, ToolOutput};
use chrono::Utc;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::context::{build_followup_messages, build_messages};

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

/// Builds [`CompletionConfig`] with `tools` populated from registered [`Tool`] instances.
fn completion_config_for_tools(tools: &[Box<dyn Tool>]) -> CompletionConfig {
    let defs: Vec<ToolDefinition> = tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        })
        .collect();
    CompletionConfig {
        tools: defs,
        ..Default::default()
    }
}

/// Owns one running agent: configuration, FSM state, policy, model backend, tool registry, chat
/// history, audit trail, optional `AKMON.md` text, and the filesystem [`Sandbox`].
pub struct AgentSession {
    config: AgentConfig,
    state: AgentState,
    policy: Arc<akmon_core::PolicyEngine>,
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    sandbox: Arc<Sandbox>,
    context: Vec<Message>,
    audit_log: Vec<AuditEvent>,
    akmon_md: Option<String>,
    /// Concatenation of all assistant [`StreamEvent::TextDelta`] chunks for this run.
    result_text: String,
    /// Completed tool calls in chronological order (for JSON run reports).
    tool_call_summaries: Vec<ToolCallSummary>,
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
    ) -> Self {
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
            result_text: String::new(),
            tool_call_summaries: Vec::new(),
        }
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
    /// When the policy is [`PolicyEngineMode::Interactive`] or [`PolicyEngineMode::AutoApproveReads`],
    /// put the session's [`mpsc::Receiver<PolicyVerdict>`] in `interactive_policy_rx` as `Some` so
    /// write confirmations can be answered; the UI must send one verdict after each
    /// [`AgentEvent::ConfirmationRequired`]. Use `&mut None` only when no interactive confirmations are
    /// possible (reads-only sessions may still use `Some` harmlessly).
    pub async fn run(
        &mut self,
        task: String,
        event_tx: mpsc::Sender<AgentEvent>,
        interactive_policy_rx: &mut Option<mpsc::Receiver<PolicyVerdict>>,
    ) -> Result<(), AgentError> {
        if !matches!(self.state, AgentState::Idle) {
            return Err(AgentError::SessionFailed {
                message: "AgentSession::run expected Idle state".into(),
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
            let tool_names: Vec<&str> = self.tools.iter().map(|t| t.name()).collect();
            let messages = if user_line_committed {
                build_followup_messages(
                    self.akmon_md.as_deref(),
                    &self.context,
                    project_root.as_str(),
                    &tool_names,
                )
            } else {
                build_messages(
                    self.akmon_md.as_deref(),
                    &self.context,
                    task.as_str(),
                    project_root.as_str(),
                    &tool_names,
                )
            };

            let completion_config = completion_config_for_tools(&self.tools);
            let mut stream: CompletionStream = match self
                .provider
                .complete(&messages, &completion_config)
                .await
            {
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
                        self.apply_event(
                            &event_tx,
                            AgentEvent::TextDelta { text },
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
                                AgentEvent::TextDelta { text: String::new() },
                                &task,
                            )
                            .await?;
                        }

                        match stop_reason {
                            StopReason::MaxTokens => {
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
                                self.apply_event(&event_tx, AgentEvent::Done, &task)
                                    .await?;
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
                                if tool_calls.is_empty() {
                                    let ae = AgentError::ModelError {
                                        message: "model returned ToolUse with no tool_calls"
                                            .into(),
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

                                for call in tool_calls {
                                    self.dispatch_one_tool_call(
                                        call,
                                        &event_tx,
                                        &task,
                                        interactive_policy_rx,
                                    )
                                    .await?;
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

    async fn dispatch_one_tool_call(
        &mut self,
        call: ModelToolCall,
        event_tx: &mpsc::Sender<AgentEvent>,
        task: &str,
        interactive_policy_rx: &mut Option<mpsc::Receiver<PolicyVerdict>>,
    ) -> Result<(), AgentError> {
        let id = call.id.clone();
        let name = call.name.clone();
        let args = call.arguments.clone();

        let Some(tool_idx) = self.tools.iter().position(|t| t.name() == name) else {
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
            self.append_tool_message(&id, &name, ToolOutput::Error {
                code: akmon_tools::ToolErrorCode::InvalidArgs,
                message: msg.clone(),
            })?;
            return Ok(());
        };

        let perms = concrete_permissions(
            self.tools[tool_idx].as_ref(),
            &name,
            &args,
            self.sandbox.primary_root(),
        );
        let session_id = self.config.session_id.to_string();

        for perm in perms {
            let allowed = match self.policy.mode() {
                PolicyEngineMode::Interactive | PolicyEngineMode::AutoApproveReads { .. } => {
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
                                        "Shell command requires confirmation.\n  Command: {command}\n  Working directory: {}",
                                        cwd.display()
                                    )
                                }
                                _ => format!("Permission required: {perm:?}"),
                            };
                            self.apply_event(
                                event_tx,
                                AgentEvent::ConfirmationRequired {
                                    description: desc.clone(),
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
                            let verdict = match rx.recv().await {
                                Some(v) => v,
                                None => {
                                    return Err(AgentError::SessionFailed {
                                        message: "policy verdict channel closed".into(),
                                    });
                                }
                            };
                            let reason: String = match verdict {
                                PolicyVerdict::Allow => "user approved (stdin)".into(),
                                PolicyVerdict::Deny => "user denied (stdin)".into(),
                            };
                            let decision = self.policy.resolve_interactive(
                                session_id.as_str(),
                                perm.clone(),
                                verdict,
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
                            self.apply_event(
                                event_tx,
                                AgentEvent::TextDelta { text: String::new() },
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
                let msg = format!("policy denied: {perm:?}");
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
                self.append_tool_message(
                    &id,
                    &name,
                    ToolOutput::Error {
                        code: akmon_tools::ToolErrorCode::PermissionDenied,
                        message: msg,
                    },
                )?;
                return Ok(());
            }
        }

        self.apply_event(
            event_tx,
            AgentEvent::ToolCallDispatched {
                id: id.clone(),
                name: name.clone(),
            },
            task,
        )
        .await?;

        let ctx = ToolContext::new(self.sandbox.as_ref().clone(), Arc::clone(&self.policy));
        let output = self.tools[tool_idx].execute(args, &ctx).await;
        let (success, message) = summarize_tool_output(&output);

        self.apply_event(
            event_tx,
            AgentEvent::ToolCallCompleted {
                id: id.clone(),
                name: name.clone(),
                success,
                message: message.clone(),
            },
            task,
        )
        .await?;

        self.append_tool_message(&id, &name, output)?;
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
        self.state = next_state_after(&self.state, &event, task)?;
        self.audit_log.push(AuditEvent::AgentStep {
            session_id: self.config.session_id.to_string(),
            timestamp: Utc::now(),
            description: event.to_string(),
        });
        if let Some(t) = tool_done {
            self.tool_call_summaries.push(t);
        }
        tx.send(event).await.map_err(|_| AgentError::SessionFailed {
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

fn map_model_error(e: ModelError) -> AgentError {
    AgentError::ModelError {
        message: e.to_string(),
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
        _ => tool.required_permissions().to_vec(),
    }
}

fn next_state_after(
    current: &AgentState,
    event: &AgentEvent,
    task: &str,
) -> Result<AgentState, AgentError> {
    match (current, event) {
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
        (AgentState::Thinking { .. }, AgentEvent::Done) => Ok(AgentState::Complete),
        (AgentState::Thinking { iteration }, AgentEvent::ToolCallDispatched { .. }) => {
            Ok(AgentState::ToolExecution {
                iteration: *iteration,
            })
        }
        (AgentState::ToolExecution { iteration }, AgentEvent::ToolCallCompleted { .. }) => {
            Ok(AgentState::Thinking {
                iteration: *iteration,
            })
        }
        (AgentState::Thinking { iteration }, AgentEvent::ToolCallCompleted { success: false, .. }) => {
            Ok(AgentState::Thinking {
                iteration: *iteration,
            })
        }
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
        _ => Err(AgentError::InvalidTransition {
            from: current.to_string(),
            to: event.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use akmon_core::PolicyEngine;
    use async_trait::async_trait;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(StubProvider::empty_end_turn()),
            vec![],
            test_sandbox(tmp.path()),
            None,
        );
        assert!(matches!(s.state(), AgentState::Idle));
    }

    #[test]
    fn iteration_limit_second_attempt_errors() {
        let config = AgentConfig {
            max_iterations: 1,
            confirmation_timeout_secs: 30,
            session_id: Uuid::nil(),
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
            4096
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
            4096
        }

        async fn complete(
            &self,
            _messages: &[Message],
            _config: &CompletionConfig,
        ) -> Result<CompletionStream, ModelError> {
            let i = self.call.fetch_add(1, Ordering::SeqCst);
            let events = self
                .sequences
                .get(i)
                .cloned()
                .unwrap_or_default();
            Ok(Box::pin(stream::iter(events)))
        }
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
        );

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        let r = session
            .run("task".into(), tx, &mut no_policy)
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
        assert!(names.iter().any(|s| s == "Done"), "expected Done, got {names:?}");
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
        let seq = SeqProvider::new(vec![
            tool_round.clone(),
            tool_round.clone(),
            tool_round,
        ]);

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 2,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        let err = session
            .run("t".into(), tx, &mut no_policy)
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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
        );

        let (tx, mut rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("hi".into(), tx, &mut no_policy)
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

        let (policy_tx, policy_rx) = mpsc::channel::<PolicyVerdict>(4);
        let _ = policy_tx.send(PolicyVerdict::Allow).await;

        let mut session = AgentSession::new(
            AgentConfig {
                max_iterations: 5,
                confirmation_timeout_secs: 30,
                session_id: Uuid::nil(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
            Arc::new(seq),
            vec![Box::new(akmon_tools::ReadFileTool::new())],
            sandbox,
            None,
        );

        let (ev_tx, _ev_rx) = mpsc::channel(64);
        let mut policy_opt = Some(policy_rx);
        session
            .run("read".into(), ev_tx, &mut policy_opt)
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
            Ok(StreamEvent::TextDelta {
                text: "hel".into(),
            }),
            Ok(StreamEvent::TextDelta {
                text: "lo".into(),
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
                session_id: Uuid::nil(),
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy)
            .await
            .expect("run");
        assert_eq!(session.result_text(), "hello");
    }

    #[tokio::test]
    async fn audit_events_roundtrip_as_jsonl_after_run() {
        use akmon_core::{write_audit_jsonl, AuditEvent};

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
            },
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
            Arc::new(seq),
            vec![],
            sandbox,
            None,
        );

        let (tx, _rx) = mpsc::channel(64);
        let mut no_policy = None;
        session
            .run("t".into(), tx, &mut no_policy)
            .await
            .expect("run");

        let audit_file = dir.path().join("audit.jsonl");
        write_audit_jsonl(&audit_file, session.audit_events()).expect("write audit");

        let contents = std::fs::read_to_string(&audit_file).expect("read audit");
        assert!(!contents.trim().is_empty());
        for line in contents.lines() {
            let parsed: AuditEvent = serde_json::from_str(line).expect("valid AuditEvent JSONL line");
            assert!(parsed.to_json().is_ok());
        }
    }
}
