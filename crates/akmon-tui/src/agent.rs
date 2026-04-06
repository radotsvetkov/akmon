//! Tokio task that runs [`AgentSession`] and bridges events to the blocking TUI thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use akmon_core::{
    write_audit_jsonl, AgentConfig, AgentEvent, McpServerConfig, PolicyEngine, PolicyEngineMode,
    PolicyVerdict, Sandbox, Secret,
};
use akmon_models::{AnthropicBackend, LlmProvider, OllamaBackend};
use akmon_query::AgentSession;
use akmon_tools::{
    discover_mcp_tools, EditTool, GitTool, ListDirectoryTool, PatchTool, ReadFileTool, SearchTool,
    SemanticSearchTool, ShellTool, WebFetchTool, WriteFileTool,
};
use tokio::sync::mpsc;
use tokio::sync::Notify;

use crate::command::UiCommand;
use crate::config::TuiLaunchConfig;

/// Message from the agent task to the terminal loop (over a `std::sync::mpsc` bridge).
#[derive(Debug)]
pub enum BridgeMsg {
    /// One streamed FSM / UI event.
    Agent(AgentEvent),
    /// The current user task finished (the session wrote audit + snapshot on the agent side).
    RunFinished,
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
    semantic: Option<crate::config::SemanticIndexSlot>,
    has_git_root: bool,
) -> Vec<Box<dyn akmon_tools::Tool>> {
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
) -> Result<(AgentSession, mpsc::Receiver<PolicyVerdict>), String> {
    let (policy_tx, policy_rx) = mpsc::channel::<PolicyVerdict>(32);
    {
        let mut guard = policy_tx_slot.lock().await;
        *guard = Some(policy_tx);
    }

    let provider: Arc<dyn LlmProvider> = match &config.anthropic_key {
        Some(key) if config.model_name.to_lowercase().starts_with("claude") => Arc::new(
            AnthropicBackend::new(Secret::new(key.clone()), config.model_name.clone()),
        ),
        _ => Arc::new(OllamaBackend::new(
            config.ollama_url.clone(),
            config.model_name.clone(),
        )),
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
        config.semantic_index.clone(),
        config.sandbox_has_git_root,
    );
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

    let agent_config = AgentConfig {
        max_iterations: config.max_iterations,
        confirmation_timeout_secs: 30,
        session_id: config.session_id,
        auto_commit: config.auto_commit,
    };

    let session = AgentSession::new(
        agent_config,
        Arc::clone(&policy),
        provider,
        tools,
        Arc::clone(&sandbox),
        config.akmon_md.clone(),
    );

    Ok((session, policy_rx))
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
    mut task_rx: mpsc::UnboundedReceiver<String>,
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
    let (mut session, policy_rx) = match build_agent_session(&initial, &policy_tx_slot).await {
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
                match build_agent_session(&cfg, &policy_tx_slot).await {
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
            task = task_rx.recv() => {
                let Some(task) = task else {
                    break;
                };
                let cfg = lock_config(&shared_config);
                interrupt.store(false, Ordering::SeqCst);
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
                        task,
                        ev_tx,
                        &mut policy_opt,
                        Some(Arc::clone(&interrupt)),
                    )
                    .await;

                let _ = forward.await;

                if let Err(e) = write_audit_jsonl(&cfg.audit_log_path, session.audit_events()) {
                    let _ = bridge_tx.send(BridgeMsg::Agent(AgentEvent::Error {
                        error: akmon_core::AgentError::SessionFailed {
                            message: format!("audit write: {e}"),
                        },
                        recoverable: true,
                    }));
                }

                let _ = run_outcome;

                let _ = bridge_tx.send(BridgeMsg::RunFinished);
            }
        }
    }
}
