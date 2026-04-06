//! Tokio task that runs [`AgentSession`] and bridges events to the blocking TUI thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use akmon_core::{
    AgentConfig, AgentEvent, McpServerConfig, PolicyEngine, PolicyEngineMode, PolicyVerdict,
    Sandbox, write_audit_jsonl,
};
use akmon_models::LlmProvider;
use akmon_query::AgentSession;
#[cfg(feature = "semantic-index")]
use akmon_tools::SemanticSearchTool;
use akmon_tools::{
    EditTool, GitTool, ListDirectoryTool, PatchTool, ReadFileTool, SearchTool, ShellTool,
    WebFetchTool, WriteFileTool, discover_mcp_tools,
};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::command::UiCommand;
use crate::config::TuiLaunchConfig;

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
    },
    /// `/init` or `/new` project tooling finished; lines are shown as system info.
    ProjectJobDone {
        /// Human-readable status lines for the transcript.
        lines: Vec<String>,
        /// When `true`, reload `AKMON.md` from disk into [`TuiLaunchConfig`] and rebuild the agent session.
        reload_akmon_md: bool,
    },
}

type PolicySenderSlot = Arc<tokio::sync::Mutex<Option<mpsc::Sender<PolicyVerdict>>>>;

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

async fn build_agent_session(
    config: &TuiLaunchConfig,
    policy_tx_slot: &PolicySenderSlot,
    plan_mode: bool,
    model_override: Option<&str>,
) -> Result<(AgentSession, mpsc::Receiver<PolicyVerdict>), String> {
    let (policy_tx, policy_rx) = mpsc::channel::<PolicyVerdict>(32);
    {
        let mut guard = policy_tx_slot.lock().await;
        *guard = Some(policy_tx);
    }

    let model = model_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| config.model_name.clone());
    let provider: Arc<dyn LlmProvider> = match config.llm_connect_for_model(model).resolve() {
        Ok(p) => p,
        Err(msg) => return Err(msg),
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

    let agent_config = AgentConfig {
        max_iterations: config.max_iterations,
        confirmation_timeout_secs: 30,
        session_id: config.session_id,
        auto_commit: if plan_mode { false } else { config.auto_commit },
    };

    let session = AgentSession::new(
        agent_config,
        Arc::clone(&policy),
        provider,
        tools,
        Arc::clone(&sandbox),
        config.akmon_md.clone(),
        plan_mode,
    );

    Ok((session, policy_rx))
}

fn apply_plan_tool_state(session: &mut AgentSession, cfg: &TuiLaunchConfig) {
    let tools = build_tool_registry(
        &cfg.shell_allow,
        cfg.web_fetch,
        #[cfg(feature = "semantic-index")]
        cfg.semantic_index.clone(),
        cfg.sandbox_has_git_root,
        true,
    );
    session.replace_tools(tools);
    session.set_plan_mode(true);
}

async fn apply_full_tools_with_mcp(
    session: &mut AgentSession,
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
    bridge_tx: std::sync::mpsc::SyncSender<BridgeMsg>,
    interrupt: Arc<AtomicBool>,
) {
    let policy_tx_slot: PolicySenderSlot = Arc::new(tokio::sync::Mutex::new(None));
    let interrupt_ui = Arc::clone(&interrupt);
    let slot_for_ui = Arc::clone(&policy_tx_slot);
    tokio::spawn(async move {
        while let Some(cmd) = ui_cmd_rx.recv().await {
            match cmd {
                UiCommand::Confirm { allow } => {
                    let v = if allow {
                        PolicyVerdict::Allow
                    } else {
                        PolicyVerdict::Deny
                    };
                    let guard = slot_for_ui.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(v).await;
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
                            let mut pop: Option<mpsc::Receiver<PolicyVerdict>> = Some(prx);
                            let _ = planner_session
                                .run(
                                    turn.task.clone(),
                                    ev_tx,
                                    &mut pop,
                                    Some(Arc::clone(&interrupt)),
                                )
                                .await;
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
                            let _ = session
                                .run(
                                    impl_task,
                                    ev_tx,
                                    &mut policy_opt,
                                    Some(Arc::clone(&interrupt)),
                                )
                                .await;
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
                        apply_plan_tool_state(&mut session, &cfg);
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

                    let run_outcome = session
                        .run(
                            turn.task.clone(),
                            ev_tx,
                            &mut policy_opt,
                            Some(Arc::clone(&interrupt)),
                        )
                        .await;

                    let _ = forward.await;

                    if turn.plan_only {
                        captured_plan = Some(session.result_text().to_string());
                    }

                    if run_outcome.is_err() {
                        captured_plan = None;
                    }
                }

                if let Err(e) = write_audit_jsonl(&cfg.audit_log_path, session.audit_events()) {
                    let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                        error: akmon_core::AgentError::SessionFailed {
                            message: format!("audit write: {e}"),
                        },
                        recoverable: true,
                    }));
                }

                let _ = bridge_tx.send(BridgeMsg::RunFinished {
                    captured_plan,
                });
            }
        }
    }
}
