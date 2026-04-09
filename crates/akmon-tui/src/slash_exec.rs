//! Execute parsed slash commands against [`TuiApp`] and shared launch configuration.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use akmon_models::{OllamaProbe, probe_ollama};
use chrono::{DateTime, Utc};
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

use crate::agent::AgentTurn;
use crate::app::{ExternalEditTarget, Overlay, TuiApp};
use crate::command::SessionSideEffect;
use crate::config::TuiLaunchConfig;
use crate::model_picker::build_model_picker_rows;
use crate::session_persist::{
    SessionSummary, default_audit_log_path, latest_dot_akmon_plan, load_session_file,
    load_session_summaries, resolve_session_id, save_session_snapshot, sessions_directory,
};
use crate::slash::{SlashCommand, parse_slash_input};
use crate::tui_project::ProjectUiJob;

/// Environment handles required to run slash commands that touch disk or the agent.
pub struct SlashEnv {
    /// Shared config kept in sync with the Tokio agent task.
    pub shared_config: Arc<Mutex<TuiLaunchConfig>>,
    /// Wakes the agent loop to rebuild [`AgentSession`] after `/reset`, `/model`, or `/resume`.
    pub reload_notify: Arc<Notify>,
    /// When `true`, `semantic_search` was enabled at startup (`--index`).
    pub index_enabled_flag: bool,
    /// Path to `.akmon/index.bin` under the active project root.
    pub index_bin_path: PathBuf,
    /// Queues `/init` and `/new` work on the async runtime.
    pub project_job_tx: mpsc::UnboundedSender<ProjectUiJob>,
    /// Queues `/implement` runs on the agent task.
    pub agent_task_tx: mpsc::UnboundedSender<AgentTurn>,
}

fn nonempty_cfg(s: &Option<String>) -> bool {
    s.as_ref().is_some_and(|x| !x.trim().is_empty())
}

/// Human-readable cost hint for `AKMON.md` size (`/doctor`).
fn akmon_md_efficiency_line(project_root: &Path) -> String {
    let path = project_root.join("AKMON.md");
    let Ok(meta) = std::fs::metadata(&path) else {
        return "AKMON.md: not found".into();
    };
    if !meta.is_file() {
        return "AKMON.md: not found".into();
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return "AKMON.md: unreadable".into();
    };
    let est = content.len() / 4;
    if est <= 500 {
        format!("✓  AKMON.md: ~{est} tokens  (efficient)")
    } else if est < 4500 {
        format!("⚠  AKMON.md: ~{est} tokens  (consider trimming — adds cost)")
    } else {
        format!("✗  AKMON.md: ~{est} tokens  (costs more than it saves)")
    }
}

fn probe_ollama_blocking(url: &str) -> OllamaProbe {
    tokio::runtime::Handle::try_current()
        .map(|h| h.block_on(probe_ollama(url)))
        .unwrap_or(OllamaProbe {
            reachable: false,
            models: vec![],
        })
}

fn refresh_app_provider_labels(app: &mut TuiApp, cfg: &TuiLaunchConfig) {
    app.provider_display_name = cfg.provider_display_name();
    app.uses_openrouter = cfg.uses_openrouter();
    app.free_local_inference = cfg.is_free_local_inference();
}

/// Outcome of handling one slash line (buffer already consumed by caller).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashHandled {
    /// Continue the TUI loop.
    Continue,
    /// Save (if possible) and exit the TUI.
    Quit,
}

/// Runs a single slash command line (including the leading `/`).
///
/// Returns [`None`] when `line` does not start with `/`.
pub fn handle_slash_line(app: &mut TuiApp, line: &str, env: &SlashEnv) -> Option<SlashHandled> {
    let s = line.trim();
    if !s.starts_with('/') {
        return None;
    }
    let parsed = parse_slash_input(s);
    let Some((cmd, arg)) = parsed else {
        app.push_system_info("Unknown or invalid slash command.".into());
        app.overlay = Overlay::None;
        return Some(SlashHandled::Continue);
    };
    Some(dispatch(app, cmd, arg, env))
}

fn dispatch(
    app: &mut TuiApp,
    cmd: &'static SlashCommand,
    arg: Option<&str>,
    env: &SlashEnv,
) -> SlashHandled {
    match cmd.name {
        "help" => {
            app.overlay = Overlay::Help;
            SlashHandled::Continue
        }
        "clear" => {
            let hard_specs = matches!(arg, Some(a) if a.trim() == "--hard");
            app.messages.clear();
            app.scroll_offset = 0;
            if let Some(tx) = app.session_effect_tx.as_ref() {
                let _ = tx.send(SessionSideEffect::ClearAgentContext { hard_specs });
            }
            app.push_system_info(if hard_specs {
                "Cleared on-screen history. Requested agent context clear and `.akmon/specs/*.md` deletion (AKMON.md kept)."
                    .into()
            } else {
                "Cleared on-screen history. Requested agent context clear; AKMON.md and specs on disk preserved."
                    .into()
            });
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "reset" => {
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before starting a new session.".into(),
                );
                return SlashHandled::Continue;
            }
            let cfg_snapshot = match env.shared_config.lock() {
                Ok(g) => g.clone(),
                Err(e) => e.into_inner().clone(),
            };
            if let Err(e) = save_session_snapshot(app, &cfg_snapshot, app.session_started_at, None)
            {
                app.push_system_info(format!("Could not save session before /reset: {e}"));
            }
            let new_id = Uuid::new_v4();
            let audit = default_audit_log_path(&app.project_root, new_id);
            {
                let mut g = match env.shared_config.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                g.session_id = new_id;
                g.audit_log_path = audit.clone();
            }
            app.session_id = new_id;
            app.audit_log_path = audit;
            app.messages.clear();
            app.total_input_tokens = 0;
            app.total_cache_read_tokens = 0;
            app.total_cache_write_tokens = 0;
            app.total_output_tokens = 0;
            app.total_microcompact_cleared = 0;
            app.context_warn_80_shown = false;
            app.context_warn_90_shown = false;
            app.current_iteration = 0;
            app.session_started_at = Utc::now();
            app.session_instant = std::time::Instant::now();
            app.has_sent_first_message = false;
            app.message_count = 0;
            app.total_tool_calls = 0;
            app.successful_tool_calls = 0;
            app.failed_tool_calls = 0;
            app.files_read.clear();
            app.files_written.clear();
            app.session_touched_files.clear();
            app.pending_plan = None;
            app.latest_plan_path = None;
            app.scroll_offset = 0;
            app.overlay = Overlay::None;
            env.reload_notify.notify_one();
            app.push_system_info("New session started.".into());
            SlashHandled::Continue
        }
        "init" => {
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before running /init.".into(),
                );
                return SlashHandled::Continue;
            }
            if env.project_job_tx.send(ProjectUiJob::Init).is_err() {
                app.push_system_info("Project job channel closed.".into());
            } else {
                app.push_system_info("Analyzing project and generating AKMON.md…".into());
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "import" => {
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before running /import.".into(),
                );
                return SlashHandled::Continue;
            }
            if env.project_job_tx.send(ProjectUiJob::Import).is_err() {
                app.push_system_info("Project job channel closed.".into());
            } else {
                app.push_system_info("Running akmon import…".into());
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "export" => {
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before running /export.".into(),
                );
                return SlashHandled::Continue;
            }
            if env.project_job_tx.send(ProjectUiJob::Export).is_err() {
                app.push_system_info("Project job channel closed.".into());
            } else {
                app.push_system_info("Running akmon export --all…".into());
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "new" => {
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before running /new.".into(),
                );
                return SlashHandled::Continue;
            }
            let Some(name) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
                app.push_system_info("Usage: /new <project-name>".into());
                return SlashHandled::Continue;
            };
            if env
                .project_job_tx
                .send(ProjectUiJob::New {
                    name: name.to_string(),
                })
                .is_err()
            {
                app.push_system_info("Project job channel closed.".into());
            } else {
                app.push_system_info(format!("Scaffolding {name}/ …"));
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "sessions" => {
            open_sessions_overlay(app);
            SlashHandled::Continue
        }
        "resume" => {
            let arg = match arg.map(str::trim) {
                Some(a) if !a.is_empty() => a,
                _ => {
                    open_sessions_overlay(app);
                    return SlashHandled::Continue;
                }
            };
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before resuming a session.".into(),
                );
                return SlashHandled::Continue;
            }
            let dir = match sessions_directory() {
                Some(d) => d,
                None => {
                    app.push_system_info("HOME not set; cannot locate sessions.".into());
                    return SlashHandled::Continue;
                }
            };
            let summaries = load_session_summaries(&dir);
            let Some(full_id) = resolve_session_id(arg, &summaries) else {
                app.push_system_info(format!("Session not found: {arg}"));
                return SlashHandled::Continue;
            };
            let path = dir.join(format!("{full_id}.json"));
            match load_session_file(&path) {
                Ok(loaded) => apply_loaded_session(app, env, loaded),
                Err(e) => {
                    app.push_system_info(format!("Session not found: {arg} ({e})"));
                }
            }
            SlashHandled::Continue
        }
        "model" => {
            if let Some(name) = arg.filter(|a| !a.is_empty()) {
                {
                    let mut g = match env.shared_config.lock() {
                        Ok(g) => g,
                        Err(e) => e.into_inner(),
                    };
                    g.model_name = name.to_string();
                }
                app.model_name = name.to_string();
                let cfg_snapshot = match env.shared_config.lock() {
                    Ok(g) => g.clone(),
                    Err(e) => e.into_inner().clone(),
                };
                refresh_app_provider_labels(app, &cfg_snapshot);
                env.reload_notify.notify_one();
                app.push_system_info(format!("Model changed to {name}"));
                app.overlay = Overlay::None;
            } else {
                let cfg_snapshot = match env.shared_config.lock() {
                    Ok(g) => g.clone(),
                    Err(e) => e.into_inner().clone(),
                };
                let probe = probe_ollama_blocking(&cfg_snapshot.ollama_url);
                app.ollama_probe = probe.clone();
                let rows = build_model_picker_rows(&cfg_snapshot, &probe, app.model_name.as_str());
                let selectable: Vec<usize> = rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.selectable && !r.section_header)
                    .map(|(i, _)| i)
                    .collect();
                if selectable.is_empty() {
                    app.push_system_info(format!("Current model: {}", app.model_name));
                    app.overlay = Overlay::None;
                } else {
                    app.push_system_info(format!(
                        "Pick a model (↑↓ Enter) — current: {}",
                        app.model_name
                    ));
                    app.overlay = Overlay::ModelPicker {
                        rows,
                        selectable,
                        selected: 0,
                        scroll: 0,
                    };
                }
            }
            SlashHandled::Continue
        }
        "doctor" => {
            let cfg_snapshot = match env.shared_config.lock() {
                Ok(g) => g.clone(),
                Err(e) => e.into_inner().clone(),
            };
            let probe = probe_ollama_blocking(&cfg_snapshot.ollama_url);
            app.ollama_probe = probe.clone();
            app.push_system_info("── Doctor ──".into());
            app.push_system_info(format!(
                "Model: {} · {}",
                cfg_snapshot.model_name,
                cfg_snapshot.provider_display_name()
            ));
            app.push_system_info(format!(
                "Anthropic API key: {}",
                if nonempty_cfg(&cfg_snapshot.anthropic_key) {
                    "set"
                } else {
                    "not set"
                }
            ));
            app.push_system_info(format!(
                "OpenRouter API key: {}",
                if nonempty_cfg(&cfg_snapshot.openrouter_key) {
                    "set"
                } else {
                    "not set"
                }
            ));
            app.push_system_info(format!(
                "OpenAI API key: {}",
                if nonempty_cfg(&cfg_snapshot.openai_key) {
                    "set"
                } else {
                    "not set"
                }
            ));
            if !probe.reachable {
                app.push_system_info("✗  Ollama: not running".into());
                app.push_system_info("Install from: https://ollama.com".into());
                app.push_system_info("Then: ollama pull qwen2.5-coder:7b".into());
            } else if probe.models.is_empty() {
                app.push_system_info("○  Ollama: running, no models installed".into());
                app.push_system_info("Run: ollama pull qwen2.5-coder:7b".into());
            } else {
                app.push_system_info("●  Ollama: running".into());
                let mut sorted = probe.models.clone();
                sorted.sort_by(|a, b| a.name.cmp(&b.name));
                for m in sorted {
                    app.push_system_info(format!("{}   {}", m.name, m.display_size()));
                }
            }
            app.push_system_info(akmon_md_efficiency_line(&app.project_root));
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "index" => {
            #[cfg(feature = "semantic-index")]
            {
                if !env.index_enabled_flag {
                    app.push_system_info(
                        "Semantic index not loaded. Restart with --index to enable.".into(),
                    );
                } else {
                    match akmon_index::load_index(&env.index_bin_path) {
                        Ok(idx) => {
                            let ago = format_index_age(idx.indexed_at);
                            app.push_system_info(format!(
                                "Semantic index: {} files, {} chunks, built {ago}",
                                idx.file_count, idx.chunk_count
                            ));
                        }
                        Err(_) => {
                            app.push_system_info(
                                "Semantic index not loaded. Restart with --index to enable.".into(),
                            );
                        }
                    }
                }
            }
            #[cfg(not(feature = "semantic-index"))]
            {
                app.push_system_info(
                    "Semantic index is not available in this build (no `semantic-index` feature)."
                        .into(),
                );
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "audit" => {
            let path = app.audit_log_path.clone();
            let lines = read_audit_overlay_lines(&path);
            app.overlay = Overlay::AuditLog { lines, scroll: 0 };
            SlashHandled::Continue
        }
        "cost" => {
            app.overlay = Overlay::CostSummary;
            SlashHandled::Continue
        }
        "plan" => {
            app.plan_only_next_turn = true;
            app.push_system_info(
                "Next message runs in read-only plan mode (no file edits, shell, or git).".into(),
            );
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "architect" => {
            app.architect_next_turn = true;
            app.push_system_info(
                "Next message uses architect mode: planner model first, then your main model."
                    .into(),
            );
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "edit-plan" => {
            if app.agent_running {
                app.push_system_info("Finish or interrupt the current turn first.".into());
                return SlashHandled::Continue;
            }
            let path = app
                .latest_plan_path
                .clone()
                .or_else(|| latest_dot_akmon_plan(&app.project_root));
            match path {
                Some(p) if p.is_file() => {
                    app.pending_external_edit = Some(ExternalEditTarget::Plan(p));
                    app.push_system_info(
                        "Opening plan in $EDITOR — save and exit to return to Akmon.".into(),
                    );
                }
                _ => app.push_system_info(
                    "No plan file found. Run /plan with a task message first.".into(),
                ),
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "view-plan" => {
            let path = app
                .latest_plan_path
                .clone()
                .or_else(|| latest_dot_akmon_plan(&app.project_root));
            match path {
                Some(p) if p.is_file() => match std::fs::read_to_string(&p) {
                    Ok(body) => {
                        let snippet: String = body.chars().take(6000).collect();
                        app.push_system_info(format!("--- {} ---\n{snippet}", p.display()));
                    }
                    Err(e) => app.push_system_info(format!("Could not read plan: {e}")),
                },
                _ => app.push_system_info("No plan file found.".into()),
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "implement" => {
            if app.agent_running {
                app.push_system_info(
                    "Finish or interrupt the current turn before /implement.".into(),
                );
                return SlashHandled::Continue;
            }
            let Some(plan) = app.pending_plan.clone() else {
                app.push_system_info(
                    "No plan stored yet. Use /plan, describe the task, then try again.".into(),
                );
                return SlashHandled::Continue;
            };
            let task = format!(
                "Implement the plan you just produced. Follow it exactly. Start with step 1.\n\n--- Plan ---\n{plan}\n---"
            );
            if env
                .agent_task_tx
                .send(AgentTurn {
                    task,
                    plan_only: false,
                    architect: false,
                })
                .is_err()
            {
                app.push_system_info("Agent task channel closed.".into());
                return SlashHandled::Continue;
            }
            app.agent_running = true;
            app.agent_activity_line = "Working — contacting model…".into();
            app.push_system_info("Implementation run queued.".into());
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "spec" => {
            let root = app.project_root.join(".akmon").join("specs");
            let Ok(rd) = std::fs::read_dir(&root) else {
                app.push_system_info(
                    "No .akmon/specs yet. CLI: akmon spec <name> \"description\"".into(),
                );
                app.overlay = Overlay::None;
                return SlashHandled::Continue;
            };
            let mut names: Vec<String> = rd
                .flatten()
                .filter_map(|e| {
                    e.path()
                        .is_dir()
                        .then(|| e.file_name().to_string_lossy().into_owned())
                })
                .collect();
            names.sort();
            if names.is_empty() {
                app.push_system_info("No specs in .akmon/specs.".into());
            } else {
                app.push_system_info(format!("Specs: {}", names.join(", ")));
            }
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "update-context" => {
            let path = app.project_root.join("AKMON.md");
            if !path.is_file() {
                app.push_system_info("AKMON.md not found in project root.".into());
                app.overlay = Overlay::None;
                return SlashHandled::Continue;
            }
            app.pending_external_edit = Some(ExternalEditTarget::AkmonMd(path));
            app.push_system_info(
                "Opening AKMON.md in $EDITOR — save and exit to return to Akmon.".into(),
            );
            app.overlay = Overlay::None;
            SlashHandled::Continue
        }
        "exit" => SlashHandled::Quit,
        _ => {
            app.push_system_info("Unknown slash command.".into());
            SlashHandled::Continue
        }
    }
}

fn open_sessions_overlay(app: &mut TuiApp) {
    let list = sessions_directory()
        .as_ref()
        .map(|d| load_session_summaries(d))
        .unwrap_or_default();
    app.overlay = Overlay::SessionList {
        sessions: list,
        selected: 0,
        scroll: 0,
    };
}

fn apply_loaded_session(
    app: &mut TuiApp,
    env: &SlashEnv,
    loaded: crate::session_persist::LoadedSession,
) {
    let audit = default_audit_log_path(&loaded.project_root, loaded.session_id);
    let ak_path = loaded.project_root.join("AKMON.md");
    let (has_akmon_md, akmon_md) = match std::fs::read_to_string(&ak_path) {
        Ok(s) => (true, Some(s)),
        Err(_) => (false, None),
    };
    {
        let mut g = match env.shared_config.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        g.session_id = loaded.session_id;
        g.project_root = loaded.project_root.clone();
        g.model_name = loaded.model_name.clone();
        g.audit_log_path = audit.clone();
        g.akmon_md = akmon_md;
        g.has_akmon_md = has_akmon_md;
    }
    app.session_id = loaded.session_id;
    app.project_root = loaded.project_root;
    app.project_name = app
        .project_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(".")
        .to_string();
    app.model_name = loaded.model_name;
    app.audit_log_path = audit;
    app.has_akmon_md = has_akmon_md;
    app.messages = loaded.messages;
    app.total_input_tokens = 0;
    app.total_cache_read_tokens = 0;
    app.total_cache_write_tokens = 0;
    app.total_output_tokens = 0;
    app.total_microcompact_cleared = 0;
    app.context_warn_80_shown = false;
    app.context_warn_90_shown = false;
    app.current_iteration = 0;
    app.session_started_at = loaded.started_at;
    app.scroll_offset = 0;
    app.overlay = Overlay::None;
    env.reload_notify.notify_one();
    let short: String = app.session_id.to_string().chars().take(8).collect();
    app.push_system_info(format!("Resumed session {short}"));
}

#[cfg(feature = "semantic-index")]
fn format_index_age(dt: DateTime<Utc>) -> String {
    let d = Utc::now().signed_duration_since(dt);
    if d.num_seconds() < 60 {
        let s = d.num_seconds().max(0);
        format!("{s}s ago")
    } else if d.num_minutes() < 60 {
        let m = d.num_minutes();
        format!("{m}m ago")
    } else if d.num_hours() < 48 {
        let h = d.num_hours();
        format!("{h}h ago")
    } else {
        let days = d.num_days();
        format!("{days}d ago")
    }
}

fn read_audit_overlay_lines(path: &std::path::Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<akmon_core::AuditEvent>(line) else {
            continue;
        };
        let (ts, kind, desc) = audit_event_parts(&ev);
        let mut d = desc;
        if d.chars().count() > 80 {
            d = d.chars().take(80).collect::<String>();
        }
        out.push(format!("{ts} {kind} {d}"));
    }
    out
}

fn audit_event_parts(ev: &akmon_core::AuditEvent) -> (String, &'static str, String) {
    use akmon_core::AuditEvent::*;
    match ev {
        PolicyEvaluation {
            timestamp,
            permission,
            verdict,
            reason,
            ..
        } => (
            timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "policy_evaluation",
            format!("{permission:?} -> {verdict:?}: {reason}"),
        ),
        ToolDispatch {
            timestamp,
            tool_name,
            input_summary,
            ..
        } => (
            timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "tool_dispatch",
            format!("{tool_name}: {input_summary}"),
        ),
        ToolOutcome {
            timestamp,
            tool_name,
            outcome,
            summary,
            ..
        } => (
            timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "tool_outcome",
            format!("{tool_name} {outcome:?}: {summary}"),
        ),
        AgentStep {
            timestamp,
            description,
            ..
        } => (
            timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "agent_step",
            description.clone(),
        ),
    }
}

/// Formats the `/cost` overlay body (excluding footer).
pub fn cost_summary_lines(app: &TuiApp) -> Vec<String> {
    let mut lines = vec![
        format!("Input tokens:       {}", app.total_input_tokens),
        format!("Cache hits:         {}", app.total_cache_read_tokens),
        format!("Cache writes:       {}", app.total_cache_write_tokens),
        format!("Output tokens:      {}", app.total_output_tokens),
        format!("Microcompact (~saved): {}", app.total_microcompact_cleared),
        "─────────────────────────".to_string(),
    ];
    let est = estimate_cost_usd(app);
    lines.push(format!("Estimated cost:     {est}"));
    lines
}

fn estimate_cost_usd(app: &TuiApp) -> String {
    match akmon_core::estimate_cost_usd(
        u64::from(app.total_input_tokens),
        u64::from(app.total_output_tokens),
        u64::from(app.total_cache_read_tokens),
        &app.model_name,
        app.uses_openrouter,
        app.free_local_inference,
    ) {
        Some(total) => format!("~${total:.4}"),
        None => "rate unknown".to_string(),
    }
}

/// Applies the highlighted `/model` row and rebuilds the agent session.
pub fn model_picker_enter(app: &mut TuiApp, env: &SlashEnv) {
    let name = match &app.overlay {
        Overlay::ModelPicker {
            rows,
            selectable,
            selected,
            ..
        } => {
            let Some(&row_i) = selectable.get(*selected) else {
                return;
            };
            let Some(row) = rows.get(row_i) else {
                return;
            };
            if row.section_header || !row.selectable {
                return;
            }
            row.label.clone()
        }
        _ => return,
    };
    {
        let mut g = match env.shared_config.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        g.model_name = name.clone();
    }
    app.model_name = name.clone();
    let cfg_snapshot = match env.shared_config.lock() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };
    refresh_app_provider_labels(app, &cfg_snapshot);
    env.reload_notify.notify_one();
    app.push_system_info(format!("Model changed to {name}"));
    app.overlay = Overlay::None;
}

/// Resumes the highlighted session from [`Overlay::SessionList`] when Enter is pressed.
pub fn session_list_enter(app: &mut TuiApp, env: &SlashEnv) {
    let Overlay::SessionList {
        sessions, selected, ..
    } = &app.overlay
    else {
        return;
    };
    if sessions.is_empty() {
        return;
    }
    let row = sessions.get(*selected);
    let Some(row) = row else {
        return;
    };
    let dir = match sessions_directory() {
        Some(d) => d,
        None => return,
    };
    let path = dir.join(format!("{}.json", row.session_id));
    if let Ok(loaded) = load_session_file(&path) {
        apply_loaded_session(app, env, loaded);
    }
    app.overlay = Overlay::None;
}

/// Formats one line for the session picker: `{date} {id8} {preview}`.
pub fn format_session_list_row(s: &SessionSummary) -> String {
    let date = DateTime::parse_from_rfc3339(&s.started_at)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| s.started_at.chars().take(16).collect());
    let id8: String = s.session_id.chars().take(8).collect();
    format!("{} {} {}", date, id8, s.first_message)
}
