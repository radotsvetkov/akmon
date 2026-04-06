//! Crossterm event loop and ratatui draw pass for the interactive UI.

use std::io::{Stdout, Write, stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::TuiApp;
use crate::agent::{AgentTurn, BridgeMsg, run_agent_loop};
use crate::app::Overlay;
use crate::command::UiCommand;
use crate::config::TuiLaunchConfig;
use crate::message::TuiMessage;
use crate::overlay::{
    draw_message_overlays, draw_slash_autocomplete, slash_autocomplete_row_count,
};
use crate::render::{message_to_lines, paint_message_viewport};
use crate::session_persist::save_session_snapshot;
use crate::slash::{matching_commands, slash_command_name_prefix};
use crate::slash_exec::{
    SlashEnv, SlashHandled, handle_slash_line, model_picker_enter, session_list_enter,
};
use crate::theme::{ACCENT, ACCENT_DIM, BORDER, FG_MUTED, FG_PRIMARY, OK_GREEN, SELECT_BG, WARN};
use crate::tui_project::ProjectUiJob;

/// Milliseconds between cursor blink ticks for streaming assistant rows.
const STREAM_BLINK_MS: u64 = 450;

/// Milliseconds between welcome-screen spark glyph swaps.
const WELCOME_SPARK_MS: u64 = 500;

/// Poll interval when waiting for input or a blink tick.
const POLL_TICK_MS: u64 = 50;

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
    let interrupt = Arc::new(AtomicBool::new(false));

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
    interrupt: Arc<AtomicBool>,
    shared_config: Arc<Mutex<TuiLaunchConfig>>,
    reload_notify: Arc<Notify>,
    project_tx: mpsc::UnboundedSender<ProjectUiJob>,
) -> Result<(), TuiRunError> {
    let mut app = TuiApp::new(config.clone());
    app.set_ui_command_tx(ui_cmd_tx);
    app.attach_runtime_handles(Arc::clone(&shared_config), Arc::clone(&reload_notify));
    let stdout_h = stdout();
    enable_raw_mode()?;
    let mut stdout_mut = stdout_h;
    execute!(stdout_mut, EnterAlternateScreen)?;
    execute!(stdout_mut, EnableMouseCapture)?;
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
    let _ = execute!(backend, DisableMouseCapture);
    let _ = execute!(backend, LeaveAlternateScreen);
    let _ = backend.flush();
    let _ = disable_raw_mode();
    result
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
    let mut running = true;

    let size = terminal.size().map_err(TuiRunError::Io)?;
    let input_h = input_area_height(app);
    let msg_h = message_viewport_height(size.height, input_h);
    app.sync_scroll_to_bottom(msg_h as usize, size.width);

    while running {
        let size = terminal.size().map_err(TuiRunError::Io)?;
        let input_h = input_area_height(app);
        let msg_h = message_viewport_height(size.height, input_h) as usize;
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
                Event::Mouse(m) => {
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        handle_input_left_click(app, m.column, m.row);
                    }
                }
                Event::Resize(w, h) => {
                    let input_h = input_area_height(app);
                    let msg_h = message_viewport_height(h, input_h);
                    app.sync_scroll_to_bottom(msg_h as usize, w);
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
            BridgeMsg::RunFinished { captured_plan } => {
                app.agent_running = false;
                interrupt.store(false, Ordering::SeqCst);
                let cfg = lock_config_clone(shared_config);
                let _ = save_session_snapshot(app, &cfg, app.session_started_at, None);
                if let Some(plan) = captured_plan {
                    app.pending_plan = Some(plan);
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

fn session_list_visible_rows(term_h: u16, input_h: u16) -> usize {
    let msg_h = message_viewport_height(term_h, input_h) as usize;
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
    let input_h = input_area_height(app);
    let msg_h = message_viewport_height(term_height, input_h) as usize;
    let list_vis = session_list_visible_rows(term_height, input_h);

    if key.kind == KeyEventKind::Release {
        return Ok(true);
    }

    if app.awaiting_confirmation {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(tx) = app.ui_command_tx.as_ref() {
                    let _ = tx.send(UiCommand::Confirm { allow: true });
                }
                app.mark_confirmation_answered(true);
                app.recompute_scroll_after_append(msg_h, term_width);
                return Ok(true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(tx) = app.ui_command_tx.as_ref() {
                    let _ = tx.send(UiCommand::Confirm { allow: false });
                }
                app.mark_confirmation_answered(false);
                app.recompute_scroll_after_append(msg_h, term_width);
                return Ok(true);
            }
            _ => return Ok(true),
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
                app.push_system_info("Interrupting after current tool…".into());
                app.recompute_scroll_after_append(msg_h, term_width);
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
            app.scroll_up(1, msg_h, term_width);
            Ok(true)
        }
        KeyCode::Down => {
            app.scroll_down(1, msg_h, term_width);
            Ok(true)
        }
        KeyCode::PageUp => {
            app.scroll_up(msg_h.max(1), msg_h, term_width);
            Ok(true)
        }
        KeyCode::PageDown => {
            app.scroll_down(msg_h.max(1), msg_h, term_width);
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
        "Keys: Enter submit · Shift+Enter newline · ←→ caret · Tab expand tool · ↑↓ PgUp/PgDn scroll · Ctrl+C interrupt · Ctrl+D exit · q exit when idle · Ctrl+/ help"
            .into(),
    );
}

fn message_viewport_height(term_height: u16, input_block_height: u16) -> u16 {
    term_height
        .saturating_sub(1 + 1 + input_block_height)
        .max(1)
}

fn input_area_height(app: &TuiApp) -> u16 {
    if app.awaiting_confirmation {
        return 5;
    }
    let ac = slash_autocomplete_row_count(app);
    let line_count = app.input_buffer.split('\n').count().clamp(1, 6);
    let body = (line_count.max(3) as u16).saturating_add(1);
    ac.saturating_add(body)
}

fn flattened_lines(app: &TuiApp, width: u16) -> Vec<Line<'static>> {
    app.messages
        .iter()
        .flat_map(|m| message_to_lines(m, width, app.stream_cursor_visible))
        .collect()
}

fn draw_frame(f: &mut ratatui::Frame<'_>, app: &mut TuiApp, area: Rect) {
    let input_h = input_area_height(app);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(input_h),
        ])
        .split(area);

    let header = build_header_line(app);
    let status = build_status_line(app, chunks[2].width);

    let msg_area = chunks[1];
    let flat = flattened_lines(app, msg_area.width);
    let viewport_h = msg_area.height as usize;
    let max_off = flat.len().saturating_sub(viewport_h);
    let scroll = app.scroll_offset.min(max_off);
    let end = (scroll + viewport_h).min(flat.len());
    let visible: Vec<Line<'static>> = if scroll < flat.len() {
        flat[scroll..end].to_vec()
    } else {
        Vec::new()
    };

    let input_text = build_input_widget(app);
    let ac_h = slash_autocomplete_row_count(app);
    let line_count = app.input_buffer.split('\n').count().clamp(1, 6);
    let input_body_h = (line_count.max(3) as u16).saturating_add(1);
    let input_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(ac_h), Constraint::Length(input_body_h)])
        .split(chunks[3]);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            " compose ",
            Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
        ));
    app.input_body_inner = Some(input_block.clone().inner(input_chunks[1]));

    f.render_widget(Paragraph::new(header), chunks[0]);
    let show_welcome = app.messages.is_empty() && !app.awaiting_confirmation;
    let show_missing_akmon_hint = show_welcome && !app.has_akmon_md;
    paint_message_viewport(
        f,
        msg_area,
        show_welcome,
        show_missing_akmon_hint,
        app.version.as_str(),
        app.project_name.as_str(),
        app.welcome_spark_phase,
        &app.context_scan,
        visible,
    );
    draw_message_overlays(f, app, msg_area);
    f.render_widget(Paragraph::new(status), chunks[2]);
    if ac_h > 0 {
        draw_slash_autocomplete(f, app, input_chunks[0]);
    }
    f.render_widget(
        Paragraph::new(input_text)
            .block(input_block)
            .wrap(Wrap { trim: false }),
        input_chunks[1],
    );
}

fn handle_input_left_click(app: &mut TuiApp, column: u16, row: u16) {
    if app.awaiting_confirmation {
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
    let b = crate::render::map_input_click_to_byte_index(&app.input_buffer, rel_row, rel_col);
    app.input_cursor = crate::render::snap_utf8_cursor(&app.input_buffer, b);
}

fn build_header_line(app: &TuiApp) -> Line<'static> {
    let project = app
        .project_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(".");
    Line::from(vec![
        Span::styled(
            "akmon",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  ·  v{}  ·  {}  ·  {}  ·  {}",
                app.version, project, app.model_name, app.mode_label
            ),
            Style::default().fg(FG_MUTED),
        ),
    ])
}

fn build_status_line(app: &TuiApp, total_width: u16) -> Line<'static> {
    let sid: String = app.session_id.to_string().chars().take(8).collect();
    let sep = Style::default().fg(BORDER);
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(sid, Style::default().fg(ACCENT_DIM)),
        Span::styled("  ·  ", sep),
        Span::styled(
            format!("in {}", app.total_input_tokens),
            Style::default().fg(FG_PRIMARY),
        ),
        Span::styled("  ·  ", sep),
    ];
    let cache_style = if app.total_cache_read_tokens > 0 {
        Style::default().fg(OK_GREEN)
    } else {
        Style::default().fg(FG_MUTED)
    };
    spans.push(Span::styled(
        format!("cache {}", app.total_cache_read_tokens),
        cache_style,
    ));
    if app.agent_running {
        spans.push(Span::styled("  ·  ", sep));
        spans.push(Span::styled(
            format!("step {}/{}", app.current_iteration, app.max_iterations),
            Style::default().fg(ACCENT_DIM),
        ));
    }
    let left_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let help = "Ctrl+/ help";
    let w = total_width as usize;
    let pad = w.saturating_sub(left_len + help.chars().count());
    spans.push(Span::raw(" ".repeat(pad.min(256))));
    spans.push(Span::styled(help, Style::default().fg(FG_MUTED)));
    Line::from(spans)
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
    if app.awaiting_confirmation {
        let desc = app
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                if let TuiMessage::Confirmation {
                    description,
                    answered,
                    ..
                } = m
                    && !answered
                {
                    return Some(description.as_str());
                }
                None
            })
            .unwrap_or("Confirmation required");
        return Text::from(vec![
            Line::from(vec![Span::styled(
                format!("⚠ {desc}"),
                Style::default().fg(WARN),
            )]),
            Line::from(vec![Span::styled(
                "[y] Allow   [n] Deny",
                Style::default().fg(FG_PRIMARY),
            )]),
        ]);
    }

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
}
