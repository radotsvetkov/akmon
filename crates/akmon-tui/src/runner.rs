//! Crossterm event loop and ratatui draw pass for the interactive UI.

use std::io::{Stdout, Write, stdout};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::TuiApp;
use crate::agent::{AgentTurn, BridgeMsg, run_agent_loop};
use crate::app::{ExternalEditTarget, Overlay};
use crate::command::{SessionSideEffect, UiCommand};
use crate::config::TuiLaunchConfig;
use crate::cost_estimate::estimate_cost_usd;
use crate::layout::{self, rect_contains};
use crate::message::TuiMessage;
use crate::overlay::{
    draw_message_overlays, draw_slash_autocomplete, draw_transcript_dim_layer,
    slash_autocomplete_row_count,
};
use crate::render::{
    CostFrag, StatusParts, context_usage_percent, flatten_transcript, paint_message_viewport,
    paint_terminal_too_small, render_confirmation_overlay, render_context_bar, render_header_bar,
    render_question_overlay, render_status_bar,
};
use crate::session_persist::{save_session_snapshot, saved_sessions_directory_empty};
use crate::slash::{matching_commands, slash_command_name_prefix};
use crate::slash_exec::{
    SlashEnv, SlashHandled, handle_slash_line, model_picker_enter, session_list_enter,
};
use crate::state::{AgentDisplayState, ConfirmChoice, OperationType};
use crate::theme::{
    ACCENT, ACCENT_DIM, BORDER, ERR, FG_MUTED, FG_PRIMARY, OK_GREEN, SELECT_BG, WARN,
};
use crate::tui_project::ProjectUiJob;

/// Milliseconds between cursor blink ticks for streaming assistant rows.
const STREAM_BLINK_MS: u64 = 450;

/// Milliseconds between welcome-screen spark glyph swaps.
const WELCOME_SPARK_MS: u64 = 500;

/// Poll interval when waiting for input or a blink tick.
const POLL_TICK_MS: u64 = 50;

/// Content rows inside the compose border (3–8), from wrap-aware height so long lines do not spill.
fn input_inner_rows_wrapped(buffer: &str, term_width: u16) -> u16 {
    let inner_w = term_width.saturating_sub(2).max(8);
    let body_rows = crate::render::input_body_row_count(buffer, inner_w).max(1) as u16;
    body_rows.saturating_add(2).clamp(3, 8)
}

fn compose_stack_inputs(app: &TuiApp, term_width: u16) -> (u16, u16) {
    let input_inner = if app.awaiting_question {
        input_inner_rows_wrapped(&app.input_buffer, term_width)
    } else if app.awaiting_confirmation || app.agent_running {
        3
    } else {
        input_inner_rows_wrapped(&app.input_buffer, term_width)
    };
    let ac = if app.awaiting_confirmation || app.awaiting_question {
        0
    } else {
        slash_autocomplete_row_count(app)
    };
    (input_inner, ac)
}

fn viewport_msg_h(term_w: u16, term_h: u16, app: &TuiApp) -> usize {
    let area = Rect::new(0, 0, term_w, term_h);
    let show_ctx = !app.session_touched_files.is_empty();
    let (input_inner, ac) = compose_stack_inputs(app, term_w);
    layout::compute_layout(area, show_ctx, input_inner, ac)
        .viewport
        .height as usize
}

/// Milliseconds between spinner frame advances (activity indicator).
const SPINNER_MS: u64 = 100;

/// Braille spinner frames for the activity indicator.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Errors returned by [`run_interactive`] / [`run_blocking`].
#[derive(Debug)]
pub enum TuiRunError {
    /// Terminal I/O failure.
    Io(std::io::Error),
    /// A Tokio task failed to join.
    Join(tokio::task::JoinError),
}

impl std::fmt::Display for TuiRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TuiRunError::Io(e) => write!(f, "{e}"),
            TuiRunError::Join(e) => write!(f, "TUI task join error: {e}"),
        }
    }
}

impl std::error::Error for TuiRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TuiRunError::Io(e) => Some(e),
            TuiRunError::Join(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for TuiRunError {
    fn from(value: std::io::Error) -> Self {
        TuiRunError::Io(value)
    }
}

/// Runs the TUI and agent concurrently (requires an active Tokio runtime).
pub async fn run_interactive(config: TuiLaunchConfig) -> Result<(), TuiRunError> {
    let (bridge_tx, bridge_rx) = std::sync::mpsc::sync_channel::<BridgeMsg>(512);
    let (task_tx, task_rx) = mpsc::unbounded_channel::<AgentTurn>();
    let (ui_cmd_tx, ui_cmd_rx) = mpsc::unbounded_channel::<UiCommand>();
    let (session_effect_tx, session_effect_rx) = mpsc::unbounded_channel::<SessionSideEffect>();
    let interrupt = Arc::new(AtomicBool::new(false));

    let ollama_bridge = bridge_tx.clone();
    let ollama_url = config.ollama_url.clone();
    tokio::spawn(async move {
        let probe = akmon_models::probe_ollama(&ollama_url).await;
        let _ = ollama_bridge.send(BridgeMsg::OllamaCatalog(probe));
    });

    let shared_config = Arc::new(Mutex::new(config.clone()));
    let reload_notify = Arc::new(Notify::new());
    let bridge_for_agent = bridge_tx.clone();
    let int_for_agent = Arc::clone(&interrupt);
    let shared_for_agent = Arc::clone(&shared_config);
    let notify_for_agent = Arc::clone(&reload_notify);
    tokio::spawn(run_agent_loop(
        shared_for_agent,
        notify_for_agent,
        task_rx,
        ui_cmd_rx,
        session_effect_rx,
        bridge_for_agent,
        int_for_agent,
    ));

    let (project_tx, mut project_rx) = mpsc::unbounded_channel::<ProjectUiJob>();
    let bridge_proj = bridge_tx.clone();
    let shared_proj = Arc::clone(&shared_config);
    tokio::spawn(async move {
        while let Some(job) = project_rx.recv().await {
            let (lines, reload) = crate::tui_project::run_project_job(job, &shared_proj).await;
            let _ = bridge_proj.send(BridgeMsg::ProjectJobDone {
                lines,
                reload_akmon_md: reload,
            });
        }
    });

    let cfg_block = config.clone();
    let ui_clone = ui_cmd_tx.clone();
    let shared_block = Arc::clone(&shared_config);
    let notify_block = Arc::clone(&reload_notify);
    match tokio::task::spawn_blocking(move || {
        run_terminal_loop(
            cfg_block,
            bridge_rx,
            task_tx,
            ui_clone,
            session_effect_tx,
            interrupt,
            shared_block,
            notify_block,
            project_tx,
        )
    })
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(TuiRunError::Join(e)),
    }
}

/// Blocks the current thread on [`run_interactive`] (for simple harnesses with a fresh runtime).
pub fn run_blocking(config: TuiLaunchConfig) -> Result<(), TuiRunError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(TuiRunError::Io)?
        .block_on(run_interactive(config))
}

#[allow(clippy::too_many_arguments)]
fn run_terminal_loop(
    config: TuiLaunchConfig,
    bridge_rx: std::sync::mpsc::Receiver<BridgeMsg>,
    task_tx: mpsc::UnboundedSender<AgentTurn>,
    ui_cmd_tx: mpsc::UnboundedSender<UiCommand>,
    session_effect_tx: mpsc::UnboundedSender<SessionSideEffect>,
    interrupt: Arc<AtomicBool>,
    shared_config: Arc<Mutex<TuiLaunchConfig>>,
    reload_notify: Arc<Notify>,
    project_tx: mpsc::UnboundedSender<ProjectUiJob>,
) -> Result<(), TuiRunError> {
    let mut app = TuiApp::new(config.clone());
    app.session_effect_tx = Some(session_effect_tx);
    app.set_ui_command_tx(ui_cmd_tx);
    app.attach_runtime_handles(Arc::clone(&shared_config), Arc::clone(&reload_notify));
    let stdout_h = stdout();
    enable_raw_mode()?;
    let mut stdout_mut = stdout_h;
    execute!(stdout_mut, EnterAlternateScreen)?;
    let _ = execute!(stdout_mut, EnableBracketedPaste);
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout_mut))?;
    let result = run_loop(
        &mut terminal,
        &mut app,
        &bridge_rx,
        &task_tx,
        &interrupt,
        &shared_config,
        &reload_notify,
        project_tx,
    );
    let backend = terminal.backend_mut();
    let _ = execute!(backend, event::DisableBracketedPaste);
    let _ = execute!(backend, DisableMouseCapture);
    let _ = execute!(backend, LeaveAlternateScreen);
    let _ = backend.flush();
    let _ = disable_raw_mode();
    print_exit_summary(&app);
    result
}

fn sync_mouse_capture(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut TuiApp,
) -> Result<(), TuiRunError> {
    if app.mouse_capture_enabled == app.mouse_capture_applied {
        return Ok(());
    }
    let backend = terminal.backend_mut();
    if app.mouse_capture_enabled {
        execute!(backend, EnableMouseCapture)?;
    } else {
        execute!(backend, DisableMouseCapture)?;
    }
    app.mouse_capture_applied = app.mouse_capture_enabled;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut TuiApp,
    bridge_rx: &std::sync::mpsc::Receiver<BridgeMsg>,
    task_tx: &mpsc::UnboundedSender<AgentTurn>,
    interrupt: &Arc<AtomicBool>,
    shared_config: &Arc<Mutex<TuiLaunchConfig>>,
    reload_notify: &Arc<Notify>,
    project_tx: mpsc::UnboundedSender<ProjectUiJob>,
) -> Result<(), TuiRunError> {
    let mut blink_accum: u64 = 0;
    let mut welcome_accum: u64 = 0;
    let mut spinner_accum: u64 = 0;
    let mut running = true;

    let size = terminal.size().map_err(TuiRunError::Io)?;
    let msg_h = viewport_msg_h(size.width, size.height, app);
    app.sync_scroll_to_bottom(msg_h, size.width);

    while running {
        sync_mouse_capture(terminal, app)?;
        let size = terminal.size().map_err(TuiRunError::Io)?;
        let msg_h = viewport_msg_h(size.width, size.height, app);
        update_slash_autocomplete_overlay(app);
        drain_bridge_messages(
            app,
            bridge_rx,
            msg_h,
            size.width,
            shared_config,
            interrupt,
            reload_notify,
        );
        if app.pending_external_edit.is_some() {
            process_pending_external_editor(terminal, app, shared_config, reload_notify)?;
            let msg_h2 = viewport_msg_h(size.width, size.height, app);
            app.recompute_scroll_after_append(msg_h2, size.width);
        }
        let area = Rect::new(0, 0, size.width, size.height);
        terminal
            .draw(|f| {
                draw_frame(f, app, area);
            })
            .map_err(TuiRunError::Io)?;

        let has_streaming = app
            .messages
            .iter()
            .any(|m| matches!(m, TuiMessage::Assistant { complete, .. } if !complete));

        let wait = Duration::from_millis(POLL_TICK_MS);
        if event::poll(wait).map_err(TuiRunError::Io)? {
            match event::read().map_err(TuiRunError::Io)? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    running = handle_key(
                        app,
                        key,
                        size.width,
                        size.height,
                        task_tx,
                        interrupt,
                        shared_config,
                        project_tx.clone(),
                    )?;
                }
                Event::Paste(text) => {
                    app.input_paste(&text);
                    app.recompute_scroll_after_append(msg_h, size.width);
                }
                Event::Mouse(m) => {
                    if !app.mouse_capture_enabled {
                        continue;
                    }
                    // Shift+click/drag should pass through for native terminal selection.
                    if m.modifiers.contains(KeyModifiers::SHIFT) {
                        continue;
                    }
                    let area_sz = Rect::new(0, 0, size.width, size.height);
                    let show_ctx = !app.session_touched_files.is_empty();
                    let (ii, ac) = compose_stack_inputs(app, size.width);
                    let lay = layout::compute_layout(area_sz, show_ctx, ii, ac);
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            handle_input_left_click(app, m.column, m.row);
                        }
                        MouseEventKind::ScrollUp => {
                            if rect_contains(lay.viewport, m.column, m.row) {
                                let ev_msg_h = lay.viewport.height as usize;
                                app.scroll_up(3, ev_msg_h, size.width);
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if rect_contains(lay.viewport, m.column, m.row) {
                                let ev_msg_h = lay.viewport.height as usize;
                                app.scroll_down(3, ev_msg_h, size.width);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Resize(w, h) => {
                    let msg_h = viewport_msg_h(w, h, app);
                    if app.auto_scroll {
                        app.sync_scroll_to_bottom(msg_h, w);
                    } else {
                        let max_off = app.max_scroll_offset(msg_h, w);
                        app.scroll_offset = app.scroll_offset.min(max_off);
                    }
                }
                _ => {}
            }
            blink_accum = 0;
        } else {
            blink_accum = blink_accum.saturating_add(POLL_TICK_MS);
            if has_streaming && blink_accum >= STREAM_BLINK_MS {
                blink_accum = 0;
                app.tick_stream_cursor();
            }
        }
        welcome_accum = welcome_accum.saturating_add(POLL_TICK_MS);
        if welcome_accum >= WELCOME_SPARK_MS {
            welcome_accum = 0;
            app.welcome_spark_phase = !app.welcome_spark_phase;
        }
        if app.agent_running {
            spinner_accum = spinner_accum.saturating_add(POLL_TICK_MS);
            if spinner_accum >= SPINNER_MS {
                spinner_accum = 0;
                app.tick_spinner();
            }
        } else {
            spinner_accum = 0;
        }
    }
    Ok(())
}

fn drain_bridge_messages(
    app: &mut TuiApp,
    bridge_rx: &std::sync::mpsc::Receiver<BridgeMsg>,
    msg_h: usize,
    width: u16,
    shared_config: &Arc<Mutex<TuiLaunchConfig>>,
    interrupt: &Arc<AtomicBool>,
    reload_notify: &Arc<Notify>,
) {
    while let Ok(msg) = bridge_rx.try_recv() {
        match msg {
            BridgeMsg::Agent(ev) => {
                app.apply_agent_event(ev);
                app.recompute_scroll_after_append(msg_h, width);
            }
            BridgeMsg::StatusInfo(msg) => {
                app.push_system_info(msg);
                app.recompute_scroll_after_append(msg_h, width);
            }
            BridgeMsg::RunFinished {
                captured_plan,
                plan_saved_path,
            } => {
                app.agent_running = false;
                app.agent_activity_line.clear();
                interrupt.store(false, Ordering::SeqCst);
                let cfg = lock_config_clone(shared_config);
                let _ = save_session_snapshot(app, &cfg, app.session_started_at, None);
                if let Some(p) = plan_saved_path {
                    app.latest_plan_path = Some(p.clone());
                    let rel = p.strip_prefix(&cfg.project_root).unwrap_or(&p);
                    app.push_system_info(format!(
                        "Plan saved to {}\n\n  /implement  execute this plan\n  /edit-plan  open in $EDITOR\n  /view-plan  show in TUI",
                        rel.display()
                    ));
                }
                if let Some(plan) = captured_plan {
                    app.pending_plan = Some(plan);
                }
                if app.pending_plan.is_some() && app.latest_plan_path.is_none() {
                    app.push_system_info(
                        "Plan complete. Review above, then type /implement to execute it or edit the plan and describe changes.".into(),
                    );
                }
                app.recompute_scroll_after_append(msg_h, width);
            }
            BridgeMsg::ProjectJobDone {
                lines,
                reload_akmon_md,
            } => {
                for line in lines {
                    app.push_system_info(line);
                }
                if reload_akmon_md {
                    let root = lock_config_clone(shared_config).project_root;
                    let path = root.join("AKMON.md");
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        if let Ok(mut g) = shared_config.lock() {
                            g.akmon_md = Some(text);
                            g.has_akmon_md = true;
                        }
                        app.has_akmon_md = true;
                        app.context_scan = akmon_core::scan_context_files(&root);
                        reload_notify.notify_one();
                    }
                }
                app.recompute_scroll_after_append(msg_h, width);
            }
            BridgeMsg::OllamaCatalog(probe) => {
                app.ollama_probe = probe;
            }
        }
    }
}

fn lock_config_clone(shared: &Arc<Mutex<TuiLaunchConfig>>) -> TuiLaunchConfig {
    match shared.lock() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    }
}

fn save_session_best_effort(app: &TuiApp, shared_config: &Arc<Mutex<TuiLaunchConfig>>) {
    let cfg = lock_config_clone(shared_config);
    let _ = save_session_snapshot(app, &cfg, app.session_started_at, None);
}

fn slash_env_for(
    app: &TuiApp,
    shared_config: &Arc<Mutex<TuiLaunchConfig>>,
    project_tx: mpsc::UnboundedSender<ProjectUiJob>,
    agent_task_tx: mpsc::UnboundedSender<AgentTurn>,
) -> SlashEnv {
    SlashEnv {
        shared_config: Arc::clone(shared_config),
        reload_notify: app
            .reload_notify
            .clone()
            .unwrap_or_else(|| Arc::new(Notify::new())),
        index_enabled_flag: app.index_enabled,
        index_bin_path: app.project_root.join(".akmon").join("index.bin"),
        project_job_tx: project_tx,
        agent_task_tx,
    }
}

fn update_slash_autocomplete_overlay(app: &mut TuiApp) {
    if matches!(
        app.overlay,
        Overlay::Help
            | Overlay::SessionList { .. }
            | Overlay::ModelPicker { .. }
            | Overlay::AuditLog { .. }
            | Overlay::CostSummary
    ) {
        return;
    }
    if app.slash_ac_suppress {
        if matches!(app.overlay, Overlay::SlashAutocomplete { .. }) {
            app.overlay = Overlay::None;
        }
        return;
    }
    match slash_command_name_prefix(&app.input_buffer) {
        None => {
            if matches!(app.overlay, Overlay::SlashAutocomplete { .. }) {
                app.overlay = Overlay::None;
            }
        }
        Some(prefix) => {
            let matches = matching_commands(prefix);
            let sig = format!(
                "{prefix}|{}|{}",
                matches.len(),
                matches.iter().map(|c| c.name).collect::<Vec<_>>().join(",")
            );
            if app.slash_ac_sig != sig {
                app.slash_ac_sig = sig;
                app.slash_ac_selected = 0;
            }
            if matches.is_empty() {
                app.slash_ac_selected = 0;
            } else {
                app.slash_ac_selected = app.slash_ac_selected.min(matches.len().saturating_sub(1));
            }
            app.overlay = Overlay::SlashAutocomplete {
                matches,
                selected: app.slash_ac_selected,
            };
        }
    }
}

fn session_list_visible_rows(term_w: u16, term_h: u16, app: &TuiApp) -> usize {
    let msg_h = viewport_msg_h(term_w, term_h, app);
    msg_h.saturating_sub(8).max(3)
}

fn model_picker_clamp(app: &mut TuiApp, visible: usize) {
    let Overlay::ModelPicker {
        rows,
        selectable,
        selected,
        scroll,
    } = &mut app.overlay
    else {
        return;
    };
    if selectable.is_empty() || rows.is_empty() {
        return;
    }
    let vis = visible.max(1);
    if *selected >= selectable.len() {
        *selected = selectable.len() - 1;
    }
    let row_i = selectable[*selected];
    if rows.len() <= vis {
        *scroll = 0;
        return;
    }
    if row_i < *scroll {
        *scroll = row_i;
    }
    if row_i >= *scroll + vis {
        *scroll = row_i + 1 - vis;
    }
    let max_scroll = rows.len().saturating_sub(vis);
    if *scroll > max_scroll {
        *scroll = max_scroll;
    }
}

fn session_list_clamp(app: &mut TuiApp, visible: usize) {
    let Overlay::SessionList {
        sessions,
        selected,
        scroll,
    } = &mut app.overlay
    else {
        return;
    };
    if sessions.is_empty() {
        return;
    }
    let vis = visible.max(1);
    if *selected >= sessions.len() {
        *selected = sessions.len() - 1;
    }
    if sessions.len() <= vis {
        *scroll = 0;
        return;
    }
    if *selected < *scroll {
        *scroll = *selected;
    }
    if *selected >= *scroll + vis {
        *scroll = (*selected + 1).saturating_sub(vis);
    }
}

fn submit_slash_autocomplete_or_line(
    app: &mut TuiApp,
    shared_config: &Arc<Mutex<TuiLaunchConfig>>,
    msg_h: usize,
    term_width: u16,
    project_tx: mpsc::UnboundedSender<ProjectUiJob>,
    agent_task_tx: mpsc::UnboundedSender<AgentTurn>,
) -> Result<bool, TuiRunError> {
    let env = slash_env_for(app, shared_config, project_tx, agent_task_tx);
    if let Overlay::SlashAutocomplete { matches, .. } = &app.overlay
        && !matches.is_empty()
    {
        let sel = app.slash_ac_selected.min(matches.len().saturating_sub(1));
        app.slash_ac_selected = sel;
        let Some(cmd) = matches.get(sel) else {
            return Ok(true);
        };
        let line = format!("/{}", cmd.name);
        app.input_buffer.clear();
        app.input_cursor = 0;
        app.overlay = Overlay::None;
        if let Some(SlashHandled::Quit) = handle_slash_line(app, &line, &env) {
            save_session_best_effort(app, shared_config);
            return Ok(false);
        }
        app.recompute_scroll_after_append(msg_h, term_width);
        return Ok(true);
    }
    let raw = app.input_buffer.trim();
    if raw.starts_with('/') {
        let line = std::mem::take(&mut app.input_buffer);
        app.input_cursor = 0;
        app.overlay = Overlay::None;
        if let Some(SlashHandled::Quit) = handle_slash_line(app, line.trim(), &env) {
            save_session_best_effort(app, shared_config);
            return Ok(false);
        }
        app.recompute_scroll_after_append(msg_h, term_width);
        return Ok(true);
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    app: &mut TuiApp,
    key: KeyEvent,
    term_width: u16,
    term_height: u16,
    task_tx: &mpsc::UnboundedSender<AgentTurn>,
    interrupt: &Arc<AtomicBool>,
    shared_config: &Arc<Mutex<TuiLaunchConfig>>,
    project_tx: mpsc::UnboundedSender<ProjectUiJob>,
) -> Result<bool, TuiRunError> {
    let msg_h = viewport_msg_h(term_width, term_height, app);
    let list_vis = session_list_visible_rows(term_width, term_height, app);

    if key.kind == KeyEventKind::Release {
        return Ok(true);
    }

    let is_ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    if !is_ctrl_c {
        app.status_flash = None;
    }

    if app.awaiting_question {
        match key.code {
            KeyCode::Enter => {
                let answer = std::mem::take(&mut app.input_buffer);
                app.input_cursor = 0;
                if let Some(tx) = app.ui_command_tx.as_ref() {
                    let _ = tx.send(UiCommand::QuestionAnswer { answer });
                }
                app.awaiting_question = false;
                app.question_prompt = None;
                app.recompute_scroll_after_append(msg_h, term_width);
                return Ok(true);
            }
            KeyCode::Esc => {
                if let Some(tx) = app.ui_command_tx.as_ref() {
                    let _ = tx.send(UiCommand::QuestionAnswer {
                        answer: String::new(),
                    });
                }
                app.input_buffer.clear();
                app.input_cursor = 0;
                app.awaiting_question = false;
                app.question_prompt = None;
                app.recompute_scroll_after_append(msg_h, term_width);
                return Ok(true);
            }
            _ => {}
        }
    }

    if key.code == KeyCode::Char('m') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.mouse_capture_enabled = !app.mouse_capture_enabled;
        app.push_system_info(if app.mouse_capture_enabled {
            "Mouse wheel ON — scrolls transcript. In many terminals hold Shift while dragging to select text.".into()
        } else {
            "Mouse wheel OFF — click/drag selects text; use ↑↓ PgUp/PgDn to scroll. Ctrl+M enables wheel again.".into()
        });
        return Ok(true);
    }

    if app.awaiting_confirmation {
        if app.confirmation_dialog.is_none()
            && let Some(TuiMessage::Confirmation {
                description,
                diff_preview,
                answered: false,
                ..
            }) = app.messages.iter().rev().find(|m| {
                matches!(
                    m,
                    TuiMessage::Confirmation {
                        answered: false,
                        ..
                    }
                )
            })
        {
            app.confirmation_dialog = Some(crate::render::dialog_from_confirmation(
                description,
                diff_preview.as_deref(),
            ));
        }
        let send_confirm = |app: &mut TuiApp,
                            allow: bool,
                            remember: bool,
                            allow_all_writes: bool,
                            shell_prefix: Option<String>| {
            if let Some(tx) = app.ui_command_tx.as_ref() {
                let _ = tx.send(UiCommand::Confirm {
                    allow,
                    remember_for_session: remember,
                    allow_all_writes_session: allow_all_writes,
                    shell_allow_prefix: if allow { shell_prefix } else { None },
                });
            }
        };
        if let Some(ref mut dlg) = app.confirmation_dialog {
            match key.code {
                KeyCode::Tab => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        dlg.cycle_choice_back();
                    } else {
                        dlg.cycle_choice();
                    }
                    return Ok(true);
                }
                KeyCode::Left => {
                    dlg.cycle_choice_back();
                    return Ok(true);
                }
                KeyCode::Right => {
                    dlg.cycle_choice();
                    return Ok(true);
                }
                KeyCode::PageUp | KeyCode::Char('k') => {
                    dlg.scroll_offset = dlg.scroll_offset.saturating_sub(2);
                    return Ok(true);
                }
                KeyCode::PageDown | KeyCode::Char('j') => {
                    dlg.scroll_offset = dlg.scroll_offset.saturating_add(2);
                    return Ok(true);
                }
                KeyCode::Enter => {
                    let shell_pfx = match &dlg.operation {
                        OperationType::RunShell { command } if dlg.broad_choice_enabled => {
                            Some(crate::render::shell_prefix_hint(command))
                        }
                        _ => None,
                    };
                    let (allow, remember, allow_all_wr, sh_pfx) = match dlg.selected_option {
                        ConfirmChoice::Allow => (true, false, false, None),
                        ConfirmChoice::AllowAlways => (true, true, false, None),
                        ConfirmChoice::AllowBroad => {
                            if dlg.broad_choice_enabled {
                                match &dlg.operation {
                                    OperationType::RunShell { .. } => {
                                        (true, false, false, shell_pfx)
                                    }
                                    _ => (true, false, true, None),
                                }
                            } else {
                                (false, false, false, None)
                            }
                        }
                        ConfirmChoice::Deny | ConfirmChoice::ViewMore => {
                            (false, false, false, None)
                        }
                    };
                    send_confirm(app, allow, remember, allow_all_wr, sh_pfx);
                    app.mark_confirmation_answered(allow);
                    app.recompute_scroll_after_append(msg_h, term_width);
                    return Ok(true);
                }
                KeyCode::Char('1') | KeyCode::Char('y') => {
                    send_confirm(app, true, false, false, None);
                    app.mark_confirmation_answered(true);
                    app.recompute_scroll_after_append(msg_h, term_width);
                    return Ok(true);
                }
                KeyCode::Char('2')
                | KeyCode::Char('Y')
                | KeyCode::Char('s')
                | KeyCode::Char('S') => {
                    send_confirm(app, true, true, false, None);
                    app.mark_confirmation_answered(true);
                    app.recompute_scroll_after_append(msg_h, term_width);
                    return Ok(true);
                }
                KeyCode::Char('p') | KeyCode::Char('P')
                    if dlg.broad_choice_enabled
                        && matches!(
                            dlg.operation,
                            OperationType::WriteFile { .. } | OperationType::EditFile { .. }
                        ) =>
                {
                    send_confirm(app, true, false, true, None);
                    app.mark_confirmation_answered(true);
                    app.recompute_scroll_after_append(msg_h, term_width);
                    return Ok(true);
                }
                KeyCode::Char('r') | KeyCode::Char('R')
                    if dlg.broad_choice_enabled
                        && matches!(dlg.operation, OperationType::RunShell { .. }) =>
                {
                    let shell_pfx = match &dlg.operation {
                        OperationType::RunShell { command } => {
                            Some(crate::render::shell_prefix_hint(command))
                        }
                        _ => None,
                    };
                    send_confirm(app, true, false, false, shell_pfx);
                    app.mark_confirmation_answered(true);
                    app.recompute_scroll_after_append(msg_h, term_width);
                    return Ok(true);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    send_confirm(app, false, false, false, None);
                    app.mark_confirmation_answered(false);
                    app.recompute_scroll_after_append(msg_h, term_width);
                    return Ok(true);
                }
                _ => return Ok(true),
            }
        } else {
            // Desync (e.g. resumed session): deny so the agent run cannot hang with a dead UI.
            if let Some(tx) = app.ui_command_tx.as_ref() {
                let _ = tx.send(UiCommand::Confirm {
                    allow: false,
                    remember_for_session: false,
                    allow_all_writes_session: false,
                    shell_allow_prefix: None,
                });
            }
            app.mark_confirmation_answered(false);
            app.push_system_info(
                "Permission prompt was missing — denied to unblock (retry the action).".into(),
            );
            return Ok(true);
        }
    }

    if matches!(app.overlay, Overlay::Help | Overlay::CostSummary) {
        app.overlay = Overlay::None;
        return Ok(true);
    }

    if matches!(app.overlay, Overlay::SessionList { .. }) {
        match key.code {
            KeyCode::Esc => {
                app.overlay = Overlay::None;
            }
            KeyCode::Enter => {
                let env = slash_env_for(app, shared_config, project_tx.clone(), task_tx.clone());
                session_list_enter(app, &env);
                app.recompute_scroll_after_append(msg_h, term_width);
            }
            KeyCode::Up => {
                if let Overlay::SessionList { selected, .. } = &mut app.overlay
                    && *selected > 0
                {
                    *selected -= 1;
                }
                session_list_clamp(app, list_vis);
            }
            KeyCode::Down => {
                if let Overlay::SessionList {
                    sessions, selected, ..
                } = &mut app.overlay
                    && *selected + 1 < sessions.len()
                {
                    *selected += 1;
                }
                session_list_clamp(app, list_vis);
            }
            _ => {}
        }
        return Ok(true);
    }

    if matches!(app.overlay, Overlay::ModelPicker { .. }) {
        match key.code {
            KeyCode::Esc => {
                app.overlay = Overlay::None;
            }
            KeyCode::Enter => {
                let env = slash_env_for(app, shared_config, project_tx.clone(), task_tx.clone());
                model_picker_enter(app, &env);
                app.recompute_scroll_after_append(msg_h, term_width);
            }
            KeyCode::Up => {
                if let Overlay::ModelPicker { selected, .. } = &mut app.overlay
                    && *selected > 0
                {
                    *selected -= 1;
                }
                model_picker_clamp(app, list_vis);
            }
            KeyCode::Down => {
                if let Overlay::ModelPicker {
                    selectable,
                    selected,
                    ..
                } = &mut app.overlay
                    && *selected + 1 < selectable.len()
                {
                    *selected += 1;
                }
                model_picker_clamp(app, list_vis);
            }
            _ => {}
        }
        return Ok(true);
    }

    if let Overlay::AuditLog { lines, scroll } = &mut app.overlay {
        match key.code {
            KeyCode::Esc => {
                app.overlay = Overlay::None;
            }
            KeyCode::Up => {
                *scroll = scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                if !lines.is_empty() {
                    *scroll = (*scroll + 1).min(lines.len().saturating_sub(1));
                }
            }
            _ => {}
        }
        return Ok(true);
    }

    if matches!(app.overlay, Overlay::SlashAutocomplete { .. }) {
        match key.code {
            KeyCode::Esc => {
                app.slash_ac_suppress = true;
                app.overlay = Overlay::None;
                return Ok(true);
            }
            KeyCode::Up => {
                if let Overlay::SlashAutocomplete { matches, .. } = &app.overlay {
                    if matches.is_empty() {
                        app.slash_ac_selected = 0;
                    } else if app.slash_ac_selected == 0 {
                        app.slash_ac_selected = matches.len() - 1;
                    } else {
                        app.slash_ac_selected -= 1;
                    }
                }
                if let Overlay::SlashAutocomplete { selected, .. } = &mut app.overlay {
                    *selected = app.slash_ac_selected;
                }
                return Ok(true);
            }
            KeyCode::Down => {
                if let Overlay::SlashAutocomplete { matches, .. } = &app.overlay {
                    if matches.is_empty() || app.slash_ac_selected + 1 >= matches.len() {
                        app.slash_ac_selected = 0;
                    } else {
                        app.slash_ac_selected += 1;
                    }
                }
                if let Overlay::SlashAutocomplete { selected, .. } = &mut app.overlay {
                    *selected = app.slash_ac_selected;
                }
                return Ok(true);
            }
            KeyCode::Enter | KeyCode::Tab => {
                return submit_slash_autocomplete_or_line(
                    app,
                    shared_config,
                    msg_h,
                    term_width,
                    project_tx.clone(),
                    task_tx.clone(),
                );
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            save_session_best_effort(app, shared_config);
            Ok(false)
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.agent_running {
                if let Some(tx) = app.ui_command_tx.as_ref() {
                    let _ = tx.send(UiCommand::Interrupt);
                }
                app.push_system_info("─ interrupted ───────────────────────────────────".into());
                app.agent_running = false;
                app.agent_activity_line.clear();
                app.recompute_scroll_after_append(msg_h, term_width);
            } else if !app.input_buffer.is_empty() {
                app.input_buffer.clear();
                app.input_cursor = 0;
            } else {
                app.status_flash = Some("use /exit to quit".into());
            }
            Ok(true)
        }
        KeyCode::Char('q') if key.modifiers.is_empty() && app.input_buffer.is_empty() => {
            if !app.agent_running {
                save_session_best_effort(app, shared_config);
                return Ok(false);
            }
            Ok(true)
        }
        KeyCode::Esc => Ok(true),
        KeyCode::Tab => {
            app.toggle_last_tool_call_expanded();
            app.recompute_scroll_after_append(msg_h, term_width);
            Ok(true)
        }
        KeyCode::Left => {
            app.input_cursor_left();
            app.recompute_scroll_after_append(msg_h, term_width);
            Ok(true)
        }
        KeyCode::Right => {
            app.input_cursor_right();
            app.recompute_scroll_after_append(msg_h, term_width);
            Ok(true)
        }
        KeyCode::Up => {
            if app.input_buffer.is_empty() || key.modifiers.contains(KeyModifiers::CONTROL) {
                let d = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    3
                } else {
                    1
                };
                app.scroll_up(d, msg_h, term_width);
            }
            Ok(true)
        }
        KeyCode::Down => {
            if app.input_buffer.is_empty() || key.modifiers.contains(KeyModifiers::CONTROL) {
                let d = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    3
                } else {
                    1
                };
                app.scroll_down(d, msg_h, term_width);
            }
            Ok(true)
        }
        KeyCode::PageUp => {
            let delta = msg_h.saturating_sub(2).max(1);
            app.scroll_up(delta, msg_h, term_width);
            Ok(true)
        }
        KeyCode::PageDown => {
            let delta = msg_h.saturating_sub(2).max(1);
            app.scroll_down(delta, msg_h, term_width);
            Ok(true)
        }
        KeyCode::Home => {
            app.scroll_offset = 0;
            app.auto_scroll = false;
            Ok(true)
        }
        KeyCode::End => {
            let max = app.max_scroll_offset(msg_h, term_width);
            app.scroll_offset = max;
            app.auto_scroll = true;
            Ok(true)
        }
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                let _ = app.input_insert('\n');
                return Ok(true);
            }
            if !submit_slash_autocomplete_or_line(
                app,
                shared_config,
                msg_h,
                term_width,
                project_tx.clone(),
                task_tx.clone(),
            )? {
                return Ok(false);
            }
            if !app.agent_running
                && let Some(task) = app.submit_user_message()
            {
                interrupt.store(false, Ordering::SeqCst);
                let plan_only = app.take_plan_only_next_turn();
                let architect = app.take_architect_next_turn();
                let _ = task_tx.send(AgentTurn {
                    task,
                    plan_only,
                    architect,
                });
                app.agent_running = true;
                app.agent_activity_line = "Working — contacting model…".into();
                app.recompute_scroll_after_append(msg_h, term_width);
            }
            Ok(true)
        }
        KeyCode::Char('?') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            show_help(app);
            app.recompute_scroll_after_append(msg_h, term_width);
            Ok(true)
        }
        KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            show_help(app);
            app.recompute_scroll_after_append(msg_h, term_width);
            Ok(true)
        }
        KeyCode::Backspace => {
            app.input_backspace();
            Ok(true)
        }
        KeyCode::Delete => {
            app.input_delete_forward();
            Ok(true)
        }
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            let _ = app.input_insert(c);
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn show_help(app: &mut TuiApp) {
    app.push_system_info(
        "Keys: Enter submit  ·  Shift+Enter newline  ·  ←→ caret  ·  Tab expand tool\n\
         Scroll: PgUp/PgDn  ·  ↑↓ when input empty  ·  Ctrl+↑↓ always  ·  mouse wheel on transcript  ·  End = latest\n\
         Approvals: Tab / ←→  ·  Enter  ·  y once  ·  2/s session  ·  p all writes  ·  r shell prefix  ·  n/Esc deny\n\
         Ctrl+C (running)=interrupt  ·  Ctrl+D / q exit  ·  Ctrl+/ this help"
            .into(),
    );
}

fn status_bar_parts(app: &TuiApp) -> StatusParts {
    let context_pct = context_usage_percent(
        app.total_input_tokens,
        app.total_cache_read_tokens,
        &app.model_name,
    );
    let (context_bar, context_color) = render_context_bar(context_pct);

    let mut sid: String = app.session_id.to_string().chars().take(8).collect();
    if let Some(ref n) = app.session_display_name {
        let t = n.trim();
        if !t.is_empty() {
            sid = format!("{t} · {sid}");
        }
    }
    let cache_style = if app.total_cache_read_tokens > 0 {
        Style::default().fg(OK_GREEN)
    } else {
        Style::default().fg(FG_MUTED)
    };

    let cost_line = if app.free_local_inference {
        Some(CostFrag {
            text: "free".into(),
            style: Style::default().fg(FG_MUTED),
        })
    } else if let Some(est) = estimate_cost_usd(
        u64::from(app.total_input_tokens),
        u64::from(app.total_output_tokens),
        u64::from(app.total_cache_read_tokens),
        &app.model_name,
        app.uses_openrouter,
        app.free_local_inference,
    ) {
        let (text, style) = session_cost_style(est);
        Some(CostFrag { text, style })
    } else if app.total_input_tokens > 0 || app.total_output_tokens > 0 {
        Some(CostFrag {
            text: "~$?".into(),
            style: Style::default().fg(FG_MUTED),
        })
    } else {
        None
    };

    let mut hint = if let Some(ref s) = app.status_flash {
        s.clone()
    } else if app.awaiting_confirmation {
        "Permission: Tab · Enter · y once · s session · p all writes · r shell prefix · n/Esc deny"
            .into()
    } else if app.agent_running {
        if let AgentDisplayState::Streaming { chars_received } = app.agent_display {
            format!("streaming {chars_received} chars")
        } else {
            "Ctrl+C interrupt".into()
        }
    } else if !app.auto_scroll {
        "End latest · Ctrl+↑↓ scroll".into()
    } else {
        "Ctrl+? help · Ctrl+↑↓ history".into()
    };

    if app.stream_cursor_visible && matches!(app.agent_display, AgentDisplayState::Streaming { .. })
    {
        hint.push_str(" ▊");
    }

    StatusParts {
        session_prefix: sid,
        input_tokens: app.total_input_tokens,
        output_tokens: app.total_output_tokens,
        context_bar,
        context_bar_style: Style::default().fg(context_color),
        cache: app.total_cache_read_tokens,
        cleared: app.total_microcompact_cleared,
        cache_style,
        cost_line,
        hint,
    }
}

fn draw_frame(f: &mut ratatui::Frame<'_>, app: &mut TuiApp, area: Rect) {
    app.sync_agent_display();
    let show_ctx = !app.session_touched_files.is_empty();
    let (input_inner, ac_h) = compose_stack_inputs(app, area.width);
    app.terminal_size = area;
    app.layout_rects = layout::compute_layout(area, show_ctx, input_inner, ac_h);

    if layout::terminal_too_small(area) {
        paint_terminal_too_small(f, area);
        return;
    }

    let clip = |r: Rect| layout::intersect_rect(area, r);
    let rects = app.layout_rects;

    render_header_bar(
        f,
        clip(rects.header),
        app.version.as_str(),
        &app.project_root,
        app.model_name.as_str(),
        app.provider_display_name.as_str(),
    );

    let vp = clip(rects.viewport);
    let flat = flatten_transcript(
        &app.messages,
        vp.width,
        app.stream_cursor_visible,
        app.light_body_text,
    );
    let viewport_h = vp.height as usize;
    let max_off = flat.len().saturating_sub(viewport_h);
    app.scroll_offset = app.scroll_offset.min(max_off);
    let scroll = app.scroll_offset;
    let end = (scroll + viewport_h).min(flat.len());
    let visible: Vec<Line<'static>> = if scroll < flat.len() {
        flat[scroll..end].to_vec()
    } else {
        Vec::new()
    };

    let show_welcome =
        app.messages.is_empty() && !app.awaiting_confirmation && !app.awaiting_question;
    let first_session_ever = saved_sessions_directory_empty();
    paint_message_viewport(
        f,
        vp,
        show_welcome,
        app.version.as_str(),
        app.project_name.as_str(),
        app.welcome_spark_phase,
        first_session_ever,
        app.has_sent_first_message,
        app.has_akmon_md,
        &app.context_scan,
        visible,
    );

    draw_message_overlays(f, app, vp);

    if app.awaiting_confirmation {
        draw_transcript_dim_layer(f, vp);
        if let Some(ref dlg) = app.confirmation_dialog {
            render_confirmation_overlay(f, vp, dlg);
        }
    } else if app.awaiting_question {
        draw_transcript_dim_layer(f, vp);
        if let Some(ref q) = app.question_prompt {
            render_question_overlay(
                f,
                vp,
                &q.question,
                &q.suggestions,
                app.input_buffer.as_str(),
            );
        }
    }

    if let Some(ctx_r) = rects.context_bar {
        let cr = clip(ctx_r);
        if let Some(ctx_line) = build_context_line(app, cr.width) {
            f.render_widget(Paragraph::new(ctx_line), cr);
        }
    }

    if let Some(ac_r) = rects.slash_autocomplete {
        draw_slash_autocomplete(f, app, clip(ac_r));
    }

    let inp = clip(rects.input);
    if app.awaiting_confirmation {
        app.input_body_inner = None;
        f.render_widget(Clear, inp);
        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "  Compose locked — use the permission dialog (Tab to choose, then Enter or Esc)",
            Style::default().fg(FG_MUTED),
        )]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(WARN)),
        );
        f.render_widget(hint, inp);
    } else if app.awaiting_question {
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(WARN))
            .title(Span::styled(
                " your answer ",
                Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
            ));
        app.input_body_inner = Some(input_block.clone().inner(inp));
        let input_text = build_input_widget(app);
        f.render_widget(
            Paragraph::new(input_text)
                .block(input_block)
                .wrap(Wrap { trim: false }),
            inp,
        );
    } else if app.agent_running {
        app.input_body_inner = None;
        let thinking = Paragraph::new(Line::from(vec![
            Span::styled(
                "> ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                SPINNER_FRAMES[app.spinner_frame as usize % SPINNER_FRAMES.len()].to_string(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", app.agent_activity_line),
                Style::default().fg(FG_MUTED),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        );
        f.render_widget(thinking, inp);
    } else {
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .title(Span::styled(
                " compose ",
                Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
            ));
        app.input_body_inner = Some(input_block.clone().inner(inp));
        let input_text = build_input_widget(app);
        f.render_widget(
            Paragraph::new(input_text)
                .block(input_block)
                .wrap(Wrap { trim: false }),
            inp,
        );
    }

    render_status_bar(f, clip(rects.status), status_bar_parts(app));
}

fn handle_input_left_click(app: &mut TuiApp, column: u16, row: u16) {
    if app.awaiting_confirmation || (app.agent_running && !app.awaiting_question) {
        return;
    }
    let Some(inner) = app.input_body_inner else {
        return;
    };
    if column < inner.x || column >= inner.x.saturating_add(inner.width) {
        return;
    }
    if row < inner.y || row >= inner.y.saturating_add(inner.height) {
        return;
    }
    let rel_col = (column - inner.x) as usize;
    let rel_row = (row - inner.y) as usize;
    let b =
        crate::render::map_input_click_wrapped(&app.input_buffer, inner.width, rel_row, rel_col);
    app.input_cursor = crate::render::snap_utf8_cursor(&app.input_buffer, b);
}

fn session_cost_style(cost: f64) -> (String, Style) {
    if cost < 0.01 {
        (
            "~$0.00".into(),
            Style::default().fg(FG_MUTED).add_modifier(Modifier::DIM),
        )
    } else if cost < 0.10 {
        (format!("~${cost:.2}"), Style::default().fg(FG_PRIMARY))
    } else if cost < 1.0 {
        (format!("~${cost:.2}"), Style::default().fg(WARN))
    } else {
        (format!("~${cost:.2}"), Style::default().fg(ERR))
    }
}

fn build_context_line(app: &TuiApp, total_width: u16) -> Option<Line<'static>> {
    if app.session_touched_files.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for p in app.session_touched_files.iter().take(2) {
        let name = Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(p.as_str());
        parts.push(name.to_string());
    }
    let extra = app.session_touched_files.len().saturating_sub(2);
    let mut text = format!("↳ context: {}", parts.join("   "));
    if extra > 0 {
        text.push_str(&format!("   +{extra} more"));
    }
    let w = total_width as usize;
    if text.chars().count() > w {
        text = crate::paths::shorten_from_left_chars(&text, w);
    }
    Some(Line::from(Span::styled(
        text,
        Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
    )))
}

fn exit_shorten_path(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
        && path.starts_with(&home)
    {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

fn exit_format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn exit_format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Plain-text summary after the TUI tears down (ANSI colors for readability).
fn print_exit_summary(app: &TuiApp) {
    let w = 52usize;
    let bar = "─".repeat(w);
    let amber = "\x1b[33m";
    let green = "\x1b[32m";
    let dim = "\x1b[2m";
    let bold = "\x1b[1m";
    let reset = "\x1b[0m";

    let wall = app.session_instant.elapsed();
    let duration_secs = wall.as_secs();
    let sid = app.session_id.to_string();
    let sid_short_len = 8usize.min(sid.len());
    let sid_short: String = sid.chars().take(sid_short_len).collect();
    let work_dir = app.project_root.to_string_lossy();
    let work_short = exit_shorten_path(&work_dir);

    let in_t = u64::from(app.total_input_tokens);
    let out_t = u64::from(app.total_output_tokens);
    let cache = u64::from(app.total_cache_read_tokens);
    let micro = u64::from(app.total_microcompact_cleared);

    let total_cost_usd = estimate_cost_usd(
        in_t,
        out_t,
        cache,
        &app.model_name,
        app.uses_openrouter,
        app.free_local_inference,
    )
    .unwrap_or(0.0);

    println!();
    println!("  {amber}▓▓ AKMON  Session complete{reset}");
    println!();
    println!("  {bar}");
    println!();

    println!("  {bold}Session{reset}");
    println!("  {:<22} {}", "ID", sid_short);
    println!(
        "  {:<22} {}",
        "Duration",
        exit_format_duration(duration_secs)
    );
    println!("  {:<22} {}", "Directory", work_short);
    println!("  {:<22} {}", "Model", app.model_name);
    println!();

    println!("  {bold}Activity{reset}");
    println!("  {:<22} {}", "Messages", app.message_count);
    println!("  {:<22} {}", "Tool calls", app.total_tool_calls);
    println!(
        "  {:<22} {green}✓ succeeded {}{}",
        " ", app.successful_tool_calls, reset
    );
    if app.failed_tool_calls > 0 {
        println!("    \x1b[31m✗ failed\x1b[0m  {}", app.failed_tool_calls);
    }
    if !app.files_written.is_empty() {
        println!("  {:<22} {}", "Files written", app.files_written.len());
        for f in &app.files_written {
            let p = exit_shorten_path(f);
            println!("  {dim}{:<22} → {}{reset}", "", p);
        }
    }
    println!();

    println!("  {bold}Tokens{reset}");
    println!("  {:<22} {}", "Input", exit_format_tokens(in_t));
    println!("  {:<22} {}", "Output", exit_format_tokens(out_t));
    if cache > 0 {
        let denom = in_t.saturating_add(cache).max(1);
        let pct = (cache * 100) / denom;
        println!(
            "  {:<22} {green}{}  ({}% cache savings){}",
            "Cache hit",
            exit_format_tokens(cache),
            pct,
            reset
        );
    }
    if micro > 0 {
        println!(
            "  {:<22} {}",
            "Microcompact (~saved)",
            exit_format_tokens(micro)
        );
    }

    let cost_line = if total_cost_usd < 0.0001 {
        format!("{green}free (local model){reset}")
    } else {
        format!("~${total_cost_usd:.4}")
    };
    println!("  {:<22} {}", "Est. cost", cost_line);

    println!();
    println!("  {bar}");
    println!();

    println!("  {dim}Audit log{reset}  .akmon/audit/{sid_short}.jsonl");
    println!();
    println!("  {amber}Goodbye!{reset}");
    println!();
}

fn process_pending_external_editor(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut TuiApp,
    shared_config: &Arc<Mutex<TuiLaunchConfig>>,
    reload_notify: &Arc<Notify>,
) -> Result<(), TuiRunError> {
    let Some(target) = app.pending_external_edit.take() else {
        return Ok(());
    };
    let path = match &target {
        ExternalEditTarget::Plan(p) | ExternalEditTarget::AkmonMd(p) => p.clone(),
    };
    let _ = terminal.show_cursor();
    let mut out = stdout();
    execute!(out, DisableMouseCapture).map_err(TuiRunError::Io)?;
    execute!(out, LeaveAlternateScreen).map_err(TuiRunError::Io)?;
    let _ = out.flush();
    disable_raw_mode()?;

    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg("exec \"${EDITOR:-vi}\" \"$@\"")
        .arg("_")
        .arg(&path)
        .status();

    enable_raw_mode()?;
    execute!(out, EnterAlternateScreen).map_err(TuiRunError::Io)?;
    if app.mouse_capture_enabled {
        execute!(out, EnableMouseCapture).map_err(TuiRunError::Io)?;
    } else {
        execute!(out, DisableMouseCapture).map_err(TuiRunError::Io)?;
    }
    app.mouse_capture_applied = app.mouse_capture_enabled;
    let _ = out.flush();
    terminal.clear().map_err(TuiRunError::Io)?;
    let _ = terminal.hide_cursor();

    match target {
        ExternalEditTarget::Plan(p) => match std::fs::read_to_string(&p) {
            Ok(body) => {
                app.pending_plan = Some(body);
                app.latest_plan_path = Some(p);
                app.push_system_info("Plan updated. /implement to proceed.".into());
            }
            Err(e) => app.push_system_info(format!("Could not reload plan: {e}")),
        },
        ExternalEditTarget::AkmonMd(_) => {
            let p = app.project_root.join("AKMON.md");
            match std::fs::read_to_string(&p) {
                Ok(text) => {
                    if let Ok(mut g) = shared_config.lock() {
                        g.akmon_md = Some(text);
                        g.has_akmon_md = true;
                    }
                    app.has_akmon_md = true;
                    app.context_scan = akmon_core::scan_context_files(&app.project_root);
                    reload_notify.notify_one();
                    app.push_system_info("AKMON.md reloaded.".into());
                }
                Err(e) => app.push_system_info(format!("Could not reload AKMON.md: {e}")),
            }
        }
    }
    Ok(())
}

fn cursor_span() -> Span<'static> {
    Span::styled("▍", Style::default().fg(ACCENT))
}

/// The glyph at the caret: keep the character visible with a bar-style highlight (Gemini-like).
fn span_char_under_caret(ch: char) -> Span<'static> {
    let st = Style::default()
        .fg(ACCENT)
        .bg(SELECT_BG)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    match ch {
        '\t' => Span::styled("→".to_string(), st),
        c if c.is_whitespace() => Span::styled('\u{00A0}'.to_string(), st),
        c => Span::styled(c.to_string(), st),
    }
}

fn build_input_widget(app: &TuiApp) -> Text<'static> {
    let buf = &app.input_buffer;
    let cur = app.input_cursor.min(buf.len());
    let mut line_spans: Vec<Span<'static>> = vec![Span::styled(
        "> ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    let mut lines_out: Vec<Line<'static>> = Vec::new();

    for (byte_i, ch) in buf.char_indices() {
        if byte_i == cur {
            if ch == '\n' {
                line_spans.push(cursor_span());
                lines_out.push(Line::from(std::mem::take(&mut line_spans)));
                line_spans.push(Span::styled("  ", Style::default().fg(ACCENT_DIM)));
            } else {
                line_spans.push(span_char_under_caret(ch));
            }
            continue;
        }
        if ch == '\n' {
            lines_out.push(Line::from(std::mem::take(&mut line_spans)));
            line_spans.push(Span::styled("  ", Style::default().fg(ACCENT_DIM)));
            continue;
        }
        line_spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(FG_PRIMARY),
        ));
    }
    if cur == buf.len() {
        line_spans.push(cursor_span());
    }
    lines_out.push(Line::from(line_spans));
    Text::from(lines_out)
}

#[cfg(test)]
mod input_mouse_tests {
    use std::path::Path;

    use ratatui::layout::Rect;
    use uuid::Uuid;

    use super::handle_input_left_click;
    use crate::TuiApp;
    use crate::config::TuiLaunchConfig;

    fn cfg() -> TuiLaunchConfig {
        TuiLaunchConfig {
            version: "t".into(),
            project_root: Path::new("/tmp/p").to_path_buf(),
            model_name: "m".into(),
            mode_label: "INTERACTIVE".into(),
            session_id: Uuid::nil(),
            max_iterations: 5,
            index_enabled: false,
            anthropic_key: None,
            openrouter_key: None,
            openai_key: None,
            groq_key: None,
            azure_endpoint: None,
            azure_key: None,
            azure_api_version: "2024-02-01".into(),
            bedrock: false,
            aws_region: "us-east-1".into(),
            openai_compatible_url: None,
            openai_compatible_key: None,
            ollama_url: "http://x".into(),
            shell_allow: Vec::new(),
            web_fetch: false,
            yes_web: false,
            auto_yes: false,
            mcp_servers: Vec::new(),
            audit_log_path: Path::new("/a.jsonl").to_path_buf(),
            akmon_md: None,
            has_akmon_md: false,
            sandbox_has_git_root: true,
            semantic_index: None,
            auto_commit: false,
            planner_model: "llama3.2".into(),
            display_theme: akmon_config::TerminalTheme::default(),
            session_display_name: None,
            resume_messages: None,
        }
    }

    #[test]
    fn mouse_click_column_5_row_0_cursor_3() {
        let mut app = TuiApp::new(cfg());
        app.input_buffer = "abcdef".into();
        app.input_body_inner = Some(Rect::new(0, 0, 80, 8));
        handle_input_left_click(&mut app, 5, 0);
        assert_eq!(app.input_cursor, 3);
    }

    #[test]
    fn mouse_click_before_prefix_cursor_0() {
        let mut app = TuiApp::new(cfg());
        app.input_buffer = "abcdef".into();
        app.input_body_inner = Some(Rect::new(0, 0, 80, 8));
        handle_input_left_click(&mut app, 0, 0);
        assert_eq!(app.input_cursor, 0);
    }

    #[test]
    fn mouse_click_past_line_end_clamped() {
        let mut app = TuiApp::new(cfg());
        app.input_buffer = "ab".into();
        app.input_body_inner = Some(Rect::new(0, 0, 80, 8));
        handle_input_left_click(&mut app, 10, 0);
        assert_eq!(app.input_cursor, 2);
    }

    #[test]
    fn cwd_shortened_starts_with_tilde_inside_home() {
        use std::path::Path;

        use crate::paths::cwd_shortened;

        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        if home.is_empty() {
            return;
        }
        let p = Path::new(&home).join("akmon-cwd-test-dir");
        let s = cwd_shortened(&p);
        assert!(
            s.starts_with("~/") || s == "~",
            "expected home-relative path to start with ~, got {s:?}"
        );
    }

    #[test]
    fn context_bar_line_counts_extra_files() {
        use super::build_context_line;

        let mut app = TuiApp::new(cfg());
        app.session_touched_files = vec!["src/x.rs".into(), "src/y.rs".into(), "src/z.rs".into()];
        let line = build_context_line(&app, 120).expect("line");
        let flat: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(flat.contains("+1 more"), "{flat}");
        assert!(flat.contains("x.rs"));
        assert!(flat.contains("y.rs"));
    }
}
