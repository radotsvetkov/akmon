//! Tokio task that runs [`AgentSession`] and bridges events to the blocking TUI thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use akmon_core::{
    AgentConfig, AgentEvent, InteractivePolicyReply, McpServerConfig, PolicyEngine,
    PolicyEngineMode, PolicyVerdict, Sandbox, save_plan_markdown, write_audit_jsonl,
};
use akmon_models::{LlmProvider, Message, MessageRole, OllamaProbe};
use akmon_query::{
    AgentSession, DefaultAgentSession, SpawnSubagentTool, SubagentRuntime, SubagentToolFactory,
    open_or_resume_default_journal_handle, write_handoff_file,
};
#[cfg(feature = "semantic-index")]
use akmon_tools::SemanticSearchTool;
use akmon_tools::{
    ApplyPatchTool, AskFollowupTool, EditTool, GitTool, ListDirectoryTool, MemoryWriteTool,
    PatchTool, ReadFileTool, ReadSpecTool, SearchTool, ShellTool, TodoWriteTool, WebFetchTool,
    WriteFileTool, WriteSpecTool, discover_mcp_tools,
};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::command::UiCommand;
use crate::config::TuiLaunchConfig;
use crate::message::TuiMessage;

fn tui_messages_to_model_messages(msgs: &[TuiMessage]) -> Vec<Message> {
    msgs.iter()
        .filter_map(|m| match m {
            TuiMessage::User { content } => Some(Message {
                role: MessageRole::User,
                content: content.clone(),
            }),
            TuiMessage::Assistant {
                content,
                complete: true,
                ..
            } => Some(Message {
                role: MessageRole::Assistant,
                content: content.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// One user-submitted agent invocation from the TUI input loop.
#[derive(Debug, Clone)]
pub struct AgentTurn {
    /// Prompt text for the model.
    pub task: String,
    /// Read-only plan pass (matches `--plan` / `/plan`).
    pub plan_only: bool,
    /// Run a cheap planner model first, then the main model (matches `--architect`).
    pub architect: bool,
}

/// Message from the agent task to the terminal loop (over a `std::sync::mpsc` bridge).
#[derive(Debug)]
pub enum BridgeMsg {
    /// One streamed FSM / UI event.
    Agent(AgentEvent),
    /// Status line for long operations (e.g. architect mode).
    StatusInfo(String),
    /// The current user task finished (the session wrote audit + snapshot on the agent side).
    RunFinished {
        /// When set, the TUI stores this as a pending implementation plan (`/implement`).
        captured_plan: Option<String>,
        /// Filesystem path when the plan body was saved under `.akmon/plans/`.
        plan_saved_path: Option<std::path::PathBuf>,
    },
    /// `/init` or `/new` project tooling finished; lines are shown as system info.
    ProjectJobDone {
        /// Human-readable status lines for the transcript.
        lines: Vec<String>,
        /// When `true`, reload `AKMON.md` from disk into [`TuiLaunchConfig`] and rebuild the agent session.
        reload_akmon_md: bool,
    },
    /// Ollama `/api/tags` probe finished (startup background refresh or explicit `/model`).
    OllamaCatalog(OllamaProbe),
}

type PolicySenderSlot = Arc<tokio::sync::Mutex<Option<mpsc::Sender<InteractivePolicyReply>>>>;
type QuestionAnswerSenderSlot = Arc<tokio::sync::Mutex<Option<mpsc::Sender<String>>>>;

fn build_tool_registry(
    shell_allow: &[String],
    web_fetch: bool,
    #[cfg(feature = "semantic-index")] semantic: Option<crate::config::SemanticIndexSlot>,
    has_git_root: bool,
    plan_mode: bool,
) -> Vec<Box<dyn akmon_tools::Tool>> {
    if plan_mode {
        let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
            Box::new(ReadFileTool::new()),
            Box::new(ListDirectoryTool::new()),
            Box::new(SearchTool::new()),
            Box::new(AskFollowupTool),
            Box::new(TodoWriteTool),
            Box::new(MemoryWriteTool),
        ];
        if web_fetch {
            tools.push(Box::new(WebFetchTool::new()));
        }
        #[cfg(feature = "semantic-index")]
        if let Some((slot, emb)) = semantic {
            tools.push(Box::new(SemanticSearchTool::new(slot, Some(emb))));
        }
        return tools;
    }
    let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
        Box::new(ReadFileTool::new()),
        Box::new(WriteFileTool::new()),
        Box::new(ListDirectoryTool::new()),
        Box::new(SearchTool::new()),
        Box::new(EditTool::new()),
        Box::new(PatchTool::new()),
        Box::new(ApplyPatchTool::new()),
        Box::new(AskFollowupTool),
        Box::new(TodoWriteTool),
        Box::new(MemoryWriteTool),
    ];
    if web_fetch {
        tools.push(Box::new(WebFetchTool::new()));
    }
    if !shell_allow.is_empty() {
        tools.push(Box::new(ShellTool::new(shell_allow.to_vec())));
    }
    #[cfg(feature = "semantic-index")]
    if let Some((slot, emb)) = semantic {
        tools.push(Box::new(SemanticSearchTool::new(slot, Some(emb))));
    }
    if has_git_root {
        tools.push(Box::new(GitTool::new()));
    }
    tools
}

fn attach_specs_subagent_tools(
    tools: &mut Vec<Box<dyn akmon_tools::Tool>>,
    cfg: &TuiLaunchConfig,
    provider: &Arc<dyn LlmProvider>,
    sandbox: &Arc<Sandbox>,
    akmon_md: &Option<String>,
    plan_mode: bool,
) {
    tools.push(Box::new(ReadSpecTool::new()));
    if !plan_mode {
        tools.push(Box::new(WriteSpecTool::new()));
    }
    let shell_allow = cfg.shell_allow.clone();
    let web_fetch = cfg.web_fetch;
    let has_git = cfg.sandbox_has_git_root;
    #[cfg(feature = "semantic-index")]
    let semantic = cfg.semantic_index.clone();
    let plan_for_sub = plan_mode;
    let factory: SubagentToolFactory = Arc::new(move || {
        #[cfg(feature = "semantic-index")]
        {
            build_tool_registry(
                &shell_allow,
                web_fetch,
                semantic.clone(),
                has_git,
                plan_for_sub,
            )
        }
        #[cfg(not(feature = "semantic-index"))]
        {
            build_tool_registry(&shell_allow, web_fetch, has_git, plan_for_sub)
        }
    });
    let rt = Arc::new(SubagentRuntime {
        provider: Arc::clone(provider),
        sandbox: Arc::clone(sandbox),
        akmon_md: akmon_md.clone(),
        plan_mode,
        confirmation_timeout_secs: 30,
        tool_factory: factory,
    });
    tools.push(Box::new(SpawnSubagentTool::new(rt)));
}

async fn build_agent_session(
    config: &TuiLaunchConfig,
    policy_tx_slot: &PolicySenderSlot,
    plan_mode: bool,
    model_override: Option<&str>,
) -> Result<(DefaultAgentSession, mpsc::Receiver<InteractivePolicyReply>), String> {
    let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
    {
        let mut guard = policy_tx_slot.lock().await;
        *guard = Some(policy_tx);
    }

    let model = model_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| config.model_name.clone());
    let provider: Arc<dyn LlmProvider> = match config.llm_connect_for_model(model).resolve() {
        Ok(p) => p,
        Err(e) => return Err(e.to_string()),
    };

    let policy_mode = if config.auto_yes {
        if config.web_fetch && config.yes_web {
            PolicyEngineMode::AutoApproveReadsAndFetch {
                confirm_writes: true,
            }
        } else {
            PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            }
        }
    } else {
        PolicyEngineMode::Interactive
    };
    let policy = Arc::new(PolicyEngine::new(policy_mode));
    let sandbox = Arc::new(Sandbox::with_git_root(
        config.project_root.clone(),
        config.sandbox_has_git_root,
    ));

    let mut tools = build_tool_registry(
        &config.shell_allow,
        config.web_fetch,
        #[cfg(feature = "semantic-index")]
        config.semantic_index.clone(),
        config.sandbox_has_git_root,
        plan_mode,
    );
    if !plan_mode {
        for url in &config.mcp_servers {
            let server = McpServerConfig {
                name: url.to_string(),
                url: url.to_string(),
                description: String::new(),
            };
            if let Ok(mcp_tools) = discover_mcp_tools(&server).await {
                for t in mcp_tools {
                    tools.push(Box::new(t));
                }
            }
        }
    }
    attach_specs_subagent_tools(
        &mut tools,
        config,
        &provider,
        &sandbox,
        &config.akmon_md,
        plan_mode,
    );

    let agent_config = AgentConfig {
        max_iterations: config.max_iterations,
        confirmation_timeout_secs: 30,
        session_id: config.session_id,
        auto_commit: if plan_mode { false } else { config.auto_commit },
        max_completion_tokens: None,
        subagent_style: false,
        max_budget_usd: None,
        fallback_model: None,
        model_estimates: config.model_estimates.clone(),
    };

    let journal =
        open_or_resume_default_journal_handle(agent_config.session_id, config.journal_resume)
            .map_err(|e| format!("journal: {e}"))?;
    let mut session = AgentSession::new(
        agent_config,
        Arc::clone(&policy),
        provider,
        tools,
        Arc::clone(&sandbox),
        config.akmon_md.clone(),
        plan_mode,
        journal,
    )
    .map_err(|e| format!("session: {e}"))?;
    if config.journal_resume {
        if let Some(msgs) = &config.resume_messages {
            let model_msgs = tui_messages_to_model_messages(msgs);
            if !model_msgs.is_empty() {
                session.restore_context_from_messages(model_msgs);
            }
        }
    }

    Ok((session, policy_rx))
}

fn apply_plan_tool_state(
    session: &mut DefaultAgentSession,
    cfg: &TuiLaunchConfig,
    provider: &Arc<dyn LlmProvider>,
    sandbox: &Arc<Sandbox>,
    akmon_md: &Option<String>,
) {
    let mut tools = build_tool_registry(
        &cfg.shell_allow,
        cfg.web_fetch,
        #[cfg(feature = "semantic-index")]
        cfg.semantic_index.clone(),
        cfg.sandbox_has_git_root,
        true,
    );
    attach_specs_subagent_tools(&mut tools, cfg, provider, sandbox, akmon_md, true);
    session.replace_tools(tools);
    session.set_plan_mode(true);
}

async fn apply_full_tools_with_mcp(
    session: &mut DefaultAgentSession,
    cfg: &TuiLaunchConfig,
) -> Result<(), String> {
    let mut tools = build_tool_registry(
        &cfg.shell_allow,
        cfg.web_fetch,
        #[cfg(feature = "semantic-index")]
        cfg.semantic_index.clone(),
        cfg.sandbox_has_git_root,
        false,
    );
    for url in &cfg.mcp_servers {
        let server = McpServerConfig {
            name: url.to_string(),
            url: url.to_string(),
            description: String::new(),
        };
        if let Ok(mcp_tools) = discover_mcp_tools(&server).await {
            for t in mcp_tools {
                tools.push(Box::new(t));
            }
        }
    }
    let prov = session.provider_arc();
    let sand = session.sandbox_arc();
    let md = session.akmon_md_cloned();
    attach_specs_subagent_tools(&mut tools, cfg, &prov, &sand, &md, false);
    session.replace_tools(tools);
    session.set_plan_mode(false);
    Ok(())
}

fn lock_config(shared: &Arc<Mutex<TuiLaunchConfig>>) -> TuiLaunchConfig {
    match shared.lock() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    }
}

/// Owns the long-lived agent session and executes each user task sequentially.
///
/// `shared_config` is updated by the TUI (`/reset`, `/model`, `/resume`); `reload_notify` triggers a
/// rebuild of [`AgentSession`] between idle turns.
pub async fn run_agent_loop(
    shared_config: Arc<Mutex<TuiLaunchConfig>>,
    reload_notify: Arc<Notify>,
    mut task_rx: mpsc::UnboundedReceiver<AgentTurn>,
    mut ui_cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
    mut session_effect_rx: mpsc::UnboundedReceiver<crate::command::SessionSideEffect>,
    bridge_tx: std::sync::mpsc::SyncSender<BridgeMsg>,
    interrupt: Arc<AtomicBool>,
) {
    let policy_tx_slot: PolicySenderSlot = Arc::new(tokio::sync::Mutex::new(None));
    let question_tx_slot: QuestionAnswerSenderSlot = Arc::new(tokio::sync::Mutex::new(None));
    let interrupt_ui = Arc::clone(&interrupt);
    let slot_for_ui = Arc::clone(&policy_tx_slot);
    let q_slot_for_ui = Arc::clone(&question_tx_slot);
    tokio::spawn(async move {
        while let Some(cmd) = ui_cmd_rx.recv().await {
            match cmd {
                UiCommand::Confirm {
                    allow,
                    remember_for_session,
                    allow_all_writes_session,
                    shell_allow_prefix,
                } => {
                    let reply = InteractivePolicyReply {
                        verdict: if allow {
                            PolicyVerdict::Allow
                        } else {
                            PolicyVerdict::Deny
                        },
                        remember_for_session: allow && remember_for_session,
                        allow_all_writes_session: allow && allow_all_writes_session,
                        shell_allow_prefix: if allow { shell_allow_prefix } else { None },
                    };
                    let guard = slot_for_ui.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(reply).await;
                    }
                }
                UiCommand::QuestionAnswer { answer } => {
                    let guard = q_slot_for_ui.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(answer).await;
                    }
                }
                UiCommand::Interrupt => interrupt_ui.store(true, Ordering::SeqCst),
            }
        }
    });

    let initial = lock_config(&shared_config);
    let (mut session, policy_rx) =
        match build_agent_session(&initial, &policy_tx_slot, false, None).await {
            Ok(x) => x,
            Err(msg) => {
                let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                    error: akmon_core::AgentError::SessionFailed { message: msg },
                    recoverable: false,
                }));
                return;
            }
        };
    let mut policy_opt = Some(policy_rx);

    loop {
        tokio::select! {
            _ = reload_notify.notified() => {
                let cfg = lock_config(&shared_config);
                match build_agent_session(&cfg, &policy_tx_slot, false, None).await {
                    Ok((s, prx)) => {
                        session = s;
                        policy_opt = Some(prx);
                    }
                    Err(msg) => {
                        let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                            error: akmon_core::AgentError::SessionFailed { message: msg },
                            recoverable: true,
                        }));
                    }
                }
            }
            eff = session_effect_rx.recv() => {
                let Some(eff) = eff else { continue };
                match eff {
                    crate::command::SessionSideEffect::ClearAgentContext { hard_specs } => {
                        let cfg = lock_config(&shared_config);
                        let _ = write_handoff_file(&session, &cfg.project_root, &cfg.model_name);
                        if let Err(e) =
                            session.clear_transcript_soft(&cfg.project_root, hard_specs)
                        {
                            let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                                error: akmon_core::AgentError::SessionFailed {
                                    message: format!("/clear: {e}"),
                                },
                                recoverable: true,
                            }));
                        } else {
                            let msg = if hard_specs {
                                "Agent transcript cleared; `.akmon/specs/*.md` removed."
                            } else {
                                "Agent transcript cleared; AKMON.md and specs on disk preserved."
                            };
                            let _ = bridge_tx.send(BridgeMsg::StatusInfo(msg.into()));
                        }
                    }
                }
            }
            recv = task_rx.recv() => {
                let Some(turn) = recv else {
                    break;
                };
                let cfg = lock_config(&shared_config);
                interrupt.store(false, Ordering::SeqCst);

                let mut captured_plan: Option<String> = None;

                if turn.architect {
                    let pm = cfg.planner_model.trim();
                    let planner_model = if pm.is_empty() {
                        "llama3.2"
                    } else {
                        pm
                    };
                    let _ = bridge_tx.send(BridgeMsg::StatusInfo(format!(
                        "Planner: {planner_model} — analyzing…"
                    )));
                    match build_agent_session(&cfg, &policy_tx_slot, true, Some(planner_model)).await {
                        Ok((mut planner_session, prx)) => {
                            let (ev_tx, mut ev_rx) = mpsc::channel::<AgentEvent>(256);
                            let bridge_ev = bridge_tx.clone();
                            let forward = tokio::spawn(async move {
                                while let Some(ev) = ev_rx.recv().await {
                                    if bridge_ev.send(BridgeMsg::Agent(ev)).is_err() {
                                        break;
                                    }
                                }
                            });
                            let mut pop: Option<mpsc::Receiver<InteractivePolicyReply>> = Some(prx);
                            let (q_tx, q_rx) = mpsc::channel::<String>(8);
                            {
                                let mut g = question_tx_slot.lock().await;
                                *g = Some(q_tx);
                            }
                            let mut q_rx_opt = Some(q_rx);
                            let _ = planner_session
                                .run(
                                    turn.task.clone(),
                                    ev_tx,
                                    &mut pop,
                                    &mut q_rx_opt,
                                    Some(Arc::clone(&interrupt)),
                                )
                                .await;
                            {
                                let mut g = question_tx_slot.lock().await;
                                *g = None;
                            }
                            let _ = forward.await;
                            let plan = planner_session.result_text().to_string();
                            let _ = bridge_tx.send(BridgeMsg::StatusInfo(format!(
                                "Implementer: {} — implementing…",
                                cfg.model_name
                            )));
                            match build_agent_session(&cfg, &policy_tx_slot, false, None).await {
                                Ok((s, prx)) => {
                                    session = s;
                                    policy_opt = Some(prx);
                                }
                                Err(msg) => {
                                    let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                                        error: akmon_core::AgentError::SessionFailed {
                                            message: msg,
                                        },
                                        recoverable: true,
                                    }));
                                    continue;
                                }
                            }
                            let impl_task = format!(
                                "Implement this plan exactly:\n\n{plan}\n\nOriginal task: {}\n\nFollow the plan step by step. Do not deviate from the plan without explaining why.",
                                turn.task
                            );
                            let (ev_tx, mut ev_rx) = mpsc::channel::<AgentEvent>(256);
                            let bridge_ev = bridge_tx.clone();
                            let forward = tokio::spawn(async move {
                                while let Some(ev) = ev_rx.recv().await {
                                    if bridge_ev.send(BridgeMsg::Agent(ev)).is_err() {
                                        break;
                                    }
                                }
                            });
                            let (q_tx, q_rx) = mpsc::channel::<String>(8);
                            {
                                let mut g = question_tx_slot.lock().await;
                                *g = Some(q_tx);
                            }
                            let mut q_rx_opt = Some(q_rx);
                            let _ = session
                                .run(
                                    impl_task,
                                    ev_tx,
                                    &mut policy_opt,
                                    &mut q_rx_opt,
                                    Some(Arc::clone(&interrupt)),
                                )
                                .await;
                            {
                                let mut g = question_tx_slot.lock().await;
                                *g = None;
                            }
                            let _ = forward.await;
                        }
                        Err(msg) => {
                            let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                                error: akmon_core::AgentError::SessionFailed { message: msg },
                                recoverable: true,
                            }));
                            continue;
                        }
                    }
                } else {
                    if turn.plan_only {
                        let prov = session.provider_arc();
                        let sand = session.sandbox_arc();
                        let md = session.akmon_md_cloned();
                        apply_plan_tool_state(&mut session, &cfg, &prov, &sand, &md);
                    } else if let Err(e) = apply_full_tools_with_mcp(&mut session, &cfg).await {
                        let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                            error: akmon_core::AgentError::SessionFailed { message: e },
                            recoverable: true,
                        }));
                        continue;
                    }
                    let (ev_tx, mut ev_rx) = mpsc::channel::<AgentEvent>(256);
                    let bridge_ev = bridge_tx.clone();
                    let forward = tokio::spawn(async move {
                        while let Some(ev) = ev_rx.recv().await {
                            if bridge_ev.send(BridgeMsg::Agent(ev)).is_err() {
                                break;
                            }
                        }
                    });

                    let (q_tx, q_rx) = mpsc::channel::<String>(8);
                    {
                        let mut g = question_tx_slot.lock().await;
                        *g = Some(q_tx);
                    }
                    let mut q_rx_opt = Some(q_rx);
                    let run_outcome = session
                        .run(
                            turn.task.clone(),
                            ev_tx,
                            &mut policy_opt,
                            &mut q_rx_opt,
                            Some(Arc::clone(&interrupt)),
                        )
                        .await;
                    {
                        let mut g = question_tx_slot.lock().await;
                        *g = None;
                    }

                    let _ = forward.await;

                    if turn.plan_only {
                        captured_plan = Some(session.result_text().to_string());
                    }

                    if run_outcome.is_err() {
                        captured_plan = None;
                    }
                }

                let plan_saved_path = match captured_plan.as_ref() {
                    Some(body) => match save_plan_markdown(&cfg.project_root, &turn.task, body) {
                        Ok(p) => Some(p),
                        Err(e) => {
                            let _ = bridge_tx.send(BridgeMsg::StatusInfo(format!(
                                "Could not save plan under .akmon/plans/: {e}"
                            )));
                            None
                        }
                    },
                    None => None,
                };

                if let Err(e) = write_audit_jsonl(&cfg.audit_log_path, session.audit_events()) {
                    let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                        error: akmon_core::AgentError::SessionFailed {
                            message: format!("audit write: {e}"),
                        },
                        recoverable: true,
                    }));
                }

                let _ = write_handoff_file(&session, &cfg.project_root, &cfg.model_name);

                let _ = bridge_tx.send(BridgeMsg::RunFinished {
                    captured_plan,
                    plan_saved_path,
                });
            }
        }
    }
}
