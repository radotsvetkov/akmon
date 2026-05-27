//! Mutable TUI state: transcript, input, scrolling, and usage counters.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ratatui::layout::Rect;

use akmon_core::{AgentEvent, ContextScan, scan_context_files};
use chrono::{DateTime, Utc};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::command::{SessionSideEffect, UiCommand};
use crate::config::TuiLaunchConfig;
use crate::layout::LayoutRects;
use crate::message::TuiMessage;
use crate::state::{
    AgentDisplayState, ComposerState, OverlayState, ProviderRuntimeState, SessionTelemetryState,
};

pub use crate::state::{ModelPickerRow, Overlay, QuestionPromptState};

/// Target file to open in the user's `EDITOR` outside the alternate-screen TUI.
#[derive(Debug, Clone)]
pub enum ExternalEditTarget {
    /// Latest saved implementation plan under `.akmon/plans/`.
    Plan(std::path::PathBuf),
    /// Project `AKMON.md`.
    AkmonMd(std::path::PathBuf),
}

/// Primary application state for the interactive terminal UI.
pub struct TuiApp {
    /// Last known full-terminal bounds (updated each frame and on resize).
    pub terminal_size: Rect,
    /// Cached root layout from [`crate::layout::compute_layout`].
    pub layout_rects: LayoutRects,
    /// Short status-bar hint after Ctrl+C on an empty buffer (cleared on next input).
    pub status_flash: Option<String>,
    /// High-level agent UI phase (spinner / streaming / …).
    pub agent_display: AgentDisplayState,
    /// Transcript rows (user, assistant, tools, …).
    pub messages: Vec<TuiMessage>,
    /// Input/composer sub-state.
    pub composer: ComposerState,
    /// First visible line index inside the flattened message list.
    pub scroll_offset: usize,
    /// When `true`, new content keeps the view pinned to the bottom.
    pub auto_scroll: bool,
    /// Session identifier (shown truncated in the status bar).
    pub session_id: Uuid,
    /// Optional session label from `--name` / `/name` (status line).
    pub session_display_name: Option<String>,
    /// Sandbox / project root.
    pub project_root: PathBuf,
    /// Last path segment of [`Self::project_root`] for branding (e.g. welcome screen).
    pub project_name: String,
    /// Model label from the CLI.
    pub model_name: String,
    /// `INTERACTIVE`, `AUTO`, etc.
    pub mode_label: String,
    /// Semver string for the header.
    pub version: String,
    /// Provider and runtime sub-state.
    pub runtime: ProviderRuntimeState,
    /// Overlay/modal and slash-autocomplete sub-state.
    pub overlays: OverlayState,
    /// Channel to the agent task (confirm / interrupt).
    pub ui_command_tx: Option<UnboundedSender<UiCommand>>,
    /// Slash `/clear` and similar session maintenance handled on the agent task.
    pub session_effect_tx: Option<UnboundedSender<SessionSideEffect>>,
    /// Wall-clock start for session snapshot metadata.
    pub session_started_at: DateTime<Utc>,
    /// Session telemetry sub-state.
    pub telemetry: SessionTelemetryState,
    /// Resolved audit JSONL path for this session (updated on `/reset` and `/resume`).
    pub audit_log_path: PathBuf,
    /// Shared launch config for the agent task (`/reset`, `/model`, `/resume`).
    pub shared_launch_config: Option<Arc<Mutex<TuiLaunchConfig>>>,
    /// Notifies the agent task to rebuild session from [`Self::shared_launch_config`].
    pub reload_notify: Option<Arc<Notify>>,
    /// Inner content rectangle of the bordered input widget (updated each frame for mouse hits).
    pub input_body_inner: Option<Rect>,
    /// Toggles welcome-screen spark glyphs (`✦` / `✧`) on a ~500 ms tick.
    pub welcome_spark_phase: bool,
    /// Mirrors [`TuiLaunchConfig::has_akmon_md`] for empty-state hints.
    pub has_akmon_md: bool,
    /// Whether `AKMON.md` is loaded for the current session context.
    pub akmon_md_loaded: bool,
    /// Whether project specs are present and may be injected.
    pub specs_loaded: bool,
    /// Other tools' context files detected at startup ([`scan_context_files`]).
    pub context_scan: ContextScan,
    /// Next message uses read-only plan mode (`/plan`).
    pub plan_only_next_turn: bool,
    /// Next message runs architect (planner + main model).
    pub architect_next_turn: bool,
    /// Last plan output for `/implement`.
    pub pending_plan: Option<String>,
    /// Path of the last plan file written under `.akmon/plans/`, if any.
    pub latest_plan_path: Option<std::path::PathBuf>,
    /// Wall-clock start for exit summary duration.
    pub session_instant: std::time::Instant,
    /// Hides first-session getting-started hints after the first user send.
    pub has_sent_first_message: bool,
    /// [`TuiLaunchConfig::model_estimates`] — context bar % and USD hints.
    pub model_estimates: Vec<akmon_core::ModelCostEstimateRow>,
    /// After `/resume`, pin the viewport to the newest line on the next redraw.
    pub resume_pin_bottom: bool,
    /// When set, the input loop opens an external editor before the next redraw.
    pub pending_external_edit: Option<ExternalEditTarget>,
    /// Rotating braille spinner frame (0..SPINNER_LEN) for the activity indicator.
    pub spinner_frame: u8,
    /// When `true`, the terminal sends mouse events (wheel scroll in the transcript).
    /// When `false`, native click/drag text selection works; scroll with ↑↓ / PgUp/PgDn.
    pub mouse_capture_enabled: bool,
    /// Last value applied to the terminal via crossterm (keeps state in sync after toggles).
    pub mouse_capture_applied: bool,
}

impl TuiApp {
    /// Builds initial state from launch metadata.
    pub fn new(config: TuiLaunchConfig) -> Self {
        let provider_display_name = config.provider_display_name();
        let uses_openrouter = config.uses_openrouter();
        let free_local_inference = config.is_free_local_inference();
        let light_body_text = config.light_body_text();
        let session_id = config.session_id;
        let session_display_name = config.session_display_name.clone();
        let started = Utc::now();
        let project_name = config
            .project_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(".")
            .to_string();
        let specs_loaded = std::fs::read_dir(config.project_root.join(".akmon/specs"))
            .ok()
            .is_some_and(|rd| {
                rd.flatten()
                    .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            });
        let context_scan = scan_context_files(&config.project_root);
        const AKMON_MD_LARGE_CHARS: usize = 2000;
        let mut messages: Vec<TuiMessage> = config.resume_messages.unwrap_or_default();
        let has_sent_first_message = messages
            .iter()
            .any(|m| matches!(m, TuiMessage::User { .. }));
        if let Some(ref md) = config.akmon_md
            && md.len() > AKMON_MD_LARGE_CHARS
        {
            let est = md.len() / 4;
            messages.push(TuiMessage::SystemInfo {
                content: format!(
                    "⚠ AKMON.md is large (~{est} tokens) — consider trimming; it is sent on every model call."
                ),
            });
        }
        Self {
            terminal_size: Rect::default(),
            layout_rects: LayoutRects {
                header: Rect::default(),
                viewport: Rect::default(),
                context_bar: None,
                slash_autocomplete: None,
                input: Rect::default(),
                status: Rect::default(),
            },
            status_flash: None,
            agent_display: AgentDisplayState::Idle,
            messages,
            composer: ComposerState::new(),
            scroll_offset: 0,
            auto_scroll: true,
            session_id,
            session_display_name,
            project_root: config.project_root,
            project_name,
            model_name: config.model_name,
            mode_label: config.mode_label,
            version: config.version,
            runtime: ProviderRuntimeState::new(
                provider_display_name,
                uses_openrouter,
                free_local_inference,
                light_body_text,
                config.max_iterations,
                config.index_enabled,
            ),
            overlays: OverlayState::new(),
            ui_command_tx: None,
            session_effect_tx: None,
            session_started_at: started,
            telemetry: SessionTelemetryState::default(),
            audit_log_path: config.audit_log_path.clone(),
            shared_launch_config: None,
            reload_notify: None,
            input_body_inner: None,
            welcome_spark_phase: false,
            has_akmon_md: config.has_akmon_md,
            akmon_md_loaded: config.has_akmon_md,
            specs_loaded,
            context_scan,
            plan_only_next_turn: false,
            architect_next_turn: false,
            pending_plan: None,
            latest_plan_path: None,
            session_instant: std::time::Instant::now(),
            has_sent_first_message,
            model_estimates: config.model_estimates.clone(),
            resume_pin_bottom: false,
            pending_external_edit: None,
            spinner_frame: 0,
            // Default ON so the wheel reaches the full transcript; Shift+drag still selects text in many terminals.
            // Ctrl+M toggles if you prefer native selection without wheel routing.
            mouse_capture_enabled: true,
            mouse_capture_applied: false,
        }
    }

    /// Advances the spinner one frame (call on a ~100 ms tick).
    pub fn tick_spinner(&mut self) {
        const LEN: u8 = 10;
        self.spinner_frame = (self.spinner_frame + 1) % LEN;
    }

    /// Consumes the `/plan` flag for the next submitted user message.
    pub fn take_plan_only_next_turn(&mut self) -> bool {
        std::mem::replace(&mut self.plan_only_next_turn, false)
    }

    /// Consumes the `/architect` flag for the next submitted user message.
    pub fn take_architect_next_turn(&mut self) -> bool {
        std::mem::replace(&mut self.architect_next_turn, false)
    }

    /// Installs the sender used for [`UiCommand`] delivery while the agent task is running.
    pub fn set_ui_command_tx(&mut self, tx: UnboundedSender<UiCommand>) {
        self.ui_command_tx = Some(tx);
    }

    /// Connects shared config + reload notify used by `/reset`, `/model`, and `/resume`.
    pub fn attach_runtime_handles(
        &mut self,
        cfg: Arc<Mutex<TuiLaunchConfig>>,
        notify: Arc<Notify>,
    ) {
        self.shared_launch_config = Some(cfg);
        self.reload_notify = Some(notify);
    }

    /// Clears the UI command channel when the agent task stops.
    pub fn clear_ui_command_tx(&mut self) {
        self.ui_command_tx = None;
    }

    /// Appends a system info line.
    pub fn push_system_info(&mut self, content: String) {
        self.messages.push(TuiMessage::SystemInfo { content });
    }

    /// Appends a user message.
    pub fn push_user(&mut self, content: String) {
        self.messages.push(TuiMessage::User { content });
    }

    /// Takes non-empty trimmed input, stores it as a user row, and returns the text for the agent.
    pub fn submit_user_message(&mut self) -> Option<String> {
        let t = self.composer.submit_trimmed()?;
        self.push_user(t.clone());
        self.has_sent_first_message = true;
        self.telemetry.message_count = self.telemetry.message_count.saturating_add(1);
        Some(t)
    }

    /// Records a sandbox-relative path in session touched-file telemetry.
    pub fn note_touched_file(&mut self, path: &str) {
        self.telemetry.note_touched_file(path);
    }

    fn tool_path_from_messages(messages: &[TuiMessage], id: &str) -> Option<String> {
        for m in messages.iter().rev() {
            if let TuiMessage::ToolCall { id: tid, args, .. } = m
                && tid == id
            {
                return args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
            }
        }
        None
    }

    /// Applies one streamed [`AgentEvent`] to the transcript and counters.
    pub fn apply_agent_event(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::TextDelta { text } => {
                if text.is_empty() {
                    return;
                }
                if self.runtime.agent_running {
                    self.runtime.agent_activity_line = "Model is responding…".into();
                }
                let append_to_last = match self.messages.last_mut() {
                    Some(TuiMessage::Assistant {
                        content,
                        complete: false,
                    }) => {
                        content.push_str(&text);
                        true
                    }
                    _ => false,
                };
                if !append_to_last {
                    self.messages.push(TuiMessage::Assistant {
                        content: text,
                        complete: false,
                    });
                }
            }
            AgentEvent::ToolCallDispatched {
                id,
                name,
                arguments,
            } => {
                self.runtime.agent_activity_line = match name.as_str() {
                    "write_file" => "Preparing file write — review approval below",
                    "edit" | "patch" => "Preparing edit — review diff when prompted",
                    "read_file" | "list_directory" | "search" | "semantic_search" => {
                        "Reading project files…"
                    }
                    "shell" => "Shell command — approval required before run",
                    "web_fetch" => "Web request — approval may be required",
                    "git" => "Git operation — approval may be required",
                    "ask_followup" => "Waiting for your answer…",
                    _ => "Running tool…",
                }
                .into();
                self.messages.push(TuiMessage::ToolCall {
                    id,
                    name,
                    args: arguments,
                    result: None,
                    success: None,
                    expanded: false,
                });
            }
            AgentEvent::ToolCallCompleted {
                id,
                name,
                success,
                message,
                ..
            } => {
                self.runtime.agent_activity_line = if success {
                    "Tool finished — continuing…"
                } else {
                    "Tool failed — model may adjust…"
                }
                .into();
                self.telemetry.total_tool_calls = self.telemetry.total_tool_calls.saturating_add(1);
                if success {
                    self.telemetry.successful_tool_calls =
                        self.telemetry.successful_tool_calls.saturating_add(1);
                    if let Some(path) = Self::tool_path_from_messages(&self.messages, &id) {
                        match name.as_str() {
                            "read_file" => self.telemetry.note_file_read(&path),
                            "write_file" | "edit" => self.telemetry.note_file_written(&path),
                            _ => {}
                        }
                        self.note_touched_file(&path);
                    }
                } else {
                    self.telemetry.failed_tool_calls =
                        self.telemetry.failed_tool_calls.saturating_add(1);
                }
                if let Some(TuiMessage::ToolCall {
                    result,
                    success: st,
                    ..
                }) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| matches!(m, TuiMessage::ToolCall { id: tid, .. } if tid == &id))
                {
                    *result = Some(message);
                    *st = Some(success);
                }
            }
            AgentEvent::QuestionRequired {
                id,
                question,
                suggestions,
            } => {
                self.runtime.agent_activity_line =
                    "Answer the question below — Enter to submit, Esc to skip".into();
                self.overlays.awaiting_question = true;
                self.composer.clear();
                self.overlays.question_prompt = Some(QuestionPromptState {
                    call_id: id,
                    question,
                    suggestions,
                });
            }
            AgentEvent::ConfirmationRequired {
                description,
                diff_preview,
            } => {
                self.runtime.agent_activity_line =
                    "Waiting for your approval — choose an option below".into();
                self.overlays.awaiting_confirmation = true;
                self.overlays.confirmation_dialog = Some(crate::render::dialog_from_confirmation(
                    &description,
                    diff_preview.as_deref(),
                ));
                self.messages.push(TuiMessage::Confirmation {
                    description,
                    diff_preview,
                    answered: false,
                    answer: None,
                });
            }
            AgentEvent::ContextSummarized {
                messages_replaced,
                tokens_freed,
            } => {
                self.push_system_info(format!(
                    "Context summarized to fit context window (messages_replaced={messages_replaced}, tokens_freed≈{tokens_freed})"
                ));
            }
            AgentEvent::StatusInfo { message } => {
                self.push_system_info(message.clone());
                if message.contains("continuing") {
                    self.runtime.agent_activity_line = "continuing response…".into();
                } else if message.starts_with('⟳') {
                    self.runtime.agent_activity_line = message.clone();
                }
            }
            AgentEvent::MicrocompactEstimate {
                estimated_tokens_cleared,
            } => {
                self.telemetry.total_microcompact_cleared = self
                    .telemetry
                    .total_microcompact_cleared
                    .saturating_add(estimated_tokens_cleared);
            }
            AgentEvent::SummarizationStarted => {
                self.push_system_info("Context summarization started…".into());
            }
            AgentEvent::UsageReport {
                input_tokens,
                output_tokens,
                cache_creation_tokens,
                cache_read_tokens,
                ..
            } => {
                if let Some(flash) = self.telemetry.apply_usage(
                    input_tokens,
                    output_tokens,
                    cache_creation_tokens,
                    cache_read_tokens,
                    &self.model_name,
                    &self.model_estimates,
                ) {
                    self.status_flash = Some(flash);
                }
            }
            AgentEvent::ProviderConfirmed { provider, .. } => {
                self.runtime.apply_provider_confirmed(&provider);
            }
            AgentEvent::IterationStarted { n, max } => {
                self.runtime.apply_iteration_started(n, max);
            }
            AgentEvent::Done => {
                // Fallback guard: the runner also flips this on RunFinished, but
                // setting it here keeps input unblocked if Done arrives early.
                self.runtime.agent_running = false;
                self.runtime.agent_activity_line.clear();
                if let Some(TuiMessage::Assistant { complete, .. }) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| matches!(m, TuiMessage::Assistant { complete: c, .. } if !c))
                {
                    *complete = true;
                } else if let Some(TuiMessage::Assistant { complete, .. }) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|m| matches!(m, TuiMessage::Assistant { .. }))
                {
                    *complete = true;
                }
            }
            AgentEvent::Error { error, .. } => {
                self.messages.push(TuiMessage::Error {
                    content: error.to_string(),
                });
            }
        }
    }

    /// Records a confirmation answer in the transcript and clears the awaiting flag.
    pub fn mark_confirmation_answered(&mut self, allowed: bool) {
        self.overlays.mark_confirmation_answered();
        if let Some(TuiMessage::Confirmation {
            answered, answer, ..
        }) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| matches!(m, TuiMessage::Confirmation { answered: a, .. } if !a))
        {
            *answered = true;
            *answer = Some(allowed);
        }
    }

    /// Toggles expand/collapse on the most recent [`TuiMessage::ToolCall`].
    pub fn toggle_last_tool_call_expanded(&mut self) {
        for m in self.messages.iter_mut().rev() {
            if let TuiMessage::ToolCall { expanded, .. } = m {
                *expanded = !*expanded;
                break;
            }
        }
    }

    /// Flattens messages to a total scrollable line count (used for paging).
    pub fn total_message_lines(&self, width: u16) -> usize {
        self.messages
            .iter()
            .map(|m| {
                crate::render::message_line_count(
                    m,
                    width,
                    self.runtime.stream_cursor_visible,
                    self.runtime.light_body_text,
                )
            })
            .sum()
    }

    /// Scrolls the message viewport up by `delta` lines; disables auto-scroll when moving away from the bottom.
    pub fn scroll_up(&mut self, delta: usize, viewport_height: usize, width: u16) {
        let max_off = self.max_scroll_offset(viewport_height, width);
        let base = if self.auto_scroll {
            max_off
        } else {
            self.scroll_offset
        };
        self.auto_scroll = false;
        self.scroll_offset = base.saturating_sub(delta);
    }

    /// Scrolls the message viewport down by `delta` lines; re-enables auto-scroll when reaching the bottom.
    pub fn scroll_down(&mut self, delta: usize, viewport_height: usize, width: u16) {
        let max_off = self.max_scroll_offset(viewport_height, width);
        let base = if self.auto_scroll {
            max_off
        } else {
            self.scroll_offset
        };
        self.scroll_offset = (base + delta).min(max_off);
        if self.scroll_offset >= max_off {
            self.auto_scroll = true;
        }
    }

    /// Returns the largest scroll offset that still fills the viewport at the bottom.
    pub fn max_scroll_offset(&self, viewport_height: usize, width: u16) -> usize {
        let total = self.total_message_lines(width);
        total.saturating_sub(viewport_height)
    }

    /// Inserts `ch` at the caret if within input limits.
    pub fn input_insert(&mut self, ch: char) -> bool {
        self.overlays.reset_slash_autocomplete_state();
        self.composer.insert(ch)
    }

    /// Inserts pasted or bulk text at the caret (used for terminal bracketed paste).
    pub fn input_paste(&mut self, text: &str) {
        self.overlays.reset_slash_autocomplete_state();
        self.composer.paste(text);
    }

    /// Removes the grapheme before the caret (ASCII slice-1 for slice 1).
    pub fn input_backspace(&mut self) {
        self.overlays.reset_slash_autocomplete_state();
        self.composer.backspace();
    }

    /// Deletes the character under the caret when present.
    pub fn input_delete_forward(&mut self) {
        self.overlays.reset_slash_autocomplete_state();
        self.composer.delete_forward();
    }

    /// Moves the caret one Unicode scalar left (no-op at start).
    pub fn input_cursor_left(&mut self) {
        self.overlays.reset_slash_autocomplete_state();
        self.composer.cursor_left();
    }

    /// Moves the caret one Unicode scalar right (no-op at end).
    pub fn input_cursor_right(&mut self) {
        self.overlays.reset_slash_autocomplete_state();
        self.composer.cursor_right();
    }

    /// Syncs scroll offset to the bottom (e.g. after resize when auto-scroll is on).
    pub fn sync_scroll_to_bottom(&mut self, viewport_height: usize, width: u16) {
        if self.auto_scroll {
            self.scroll_offset = self.max_scroll_offset(viewport_height, width);
        }
    }

    /// Flips the streaming cursor blink bit (call on a timer tick).
    pub fn tick_stream_cursor(&mut self) {
        self.runtime.tick_stream_cursor();
    }

    /// After appending transcript rows, pin the viewport to the newest line when [`Self::auto_scroll`] is enabled.
    pub fn recompute_scroll_after_append(&mut self, viewport_height: usize, width: u16) {
        if self.auto_scroll {
            self.scroll_offset = self.max_scroll_offset(viewport_height, width);
        }
    }

    /// Derives [`Self::agent_display`] from flags and the transcript tail.
    pub fn sync_agent_display(&mut self) {
        self.agent_display = if self.overlays.awaiting_confirmation {
            AgentDisplayState::WaitingForConfirmation
        } else if !self.runtime.agent_running {
            AgentDisplayState::Idle
        } else if let Some(name) = self.messages.iter().rev().find_map(|m| {
            if let TuiMessage::ToolCall {
                name,
                success: None,
                ..
            } = m
            {
                Some(name.clone())
            } else {
                None
            }
        }) {
            AgentDisplayState::CallingTool {
                tool_name: name,
                step: self.runtime.current_iteration,
            }
        } else if let Some(n) = self.messages.iter().rev().find_map(|m| {
            if let TuiMessage::Assistant {
                content,
                complete: false,
                ..
            } = m
            {
                Some(content.len() as u64)
            } else {
                None
            }
        }) {
            AgentDisplayState::Streaming { chars_received: n }
        } else {
            AgentDisplayState::Thinking
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn sample_config() -> TuiLaunchConfig {
        TuiLaunchConfig {
            version: "1.3.0-test".into(),
            project_root: Path::new("/tmp/Akmon").to_path_buf(),
            model_name: "test-model".into(),
            mode_label: "INTERACTIVE".into(),
            session_id: Uuid::nil(),
            max_iterations: 25,
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
            ollama_url: "http://localhost:11434".into(),
            shell_allow: Vec::new(),
            web_fetch: false,
            yes_web: false,
            auto_yes: false,
            mcp_servers: Vec::new(),
            audit_log_path: PathBuf::from("/tmp/audit.jsonl"),
            akmon_md: None,
            has_akmon_md: false,
            sandbox_has_git_root: true,
            semantic_index: None,
            auto_commit: false,
            planner_model: "llama3.2".into(),
            display_theme: akmon_config::TerminalTheme::default(),
            session_display_name: None,
            resume_messages: None,
            journal_resume: false,
            model_estimates: Vec::new(),
        }
    }

    #[test]
    fn new_defaults() {
        let app = TuiApp::new(sample_config());
        assert!(app.auto_scroll);
        assert!(!app.runtime.agent_running);
        assert_eq!(app.composer.cursor, 0);
        assert!(app.composer.buffer.is_empty());
        assert_eq!(app.runtime.max_iterations, 25);
        assert!(app.messages.is_empty());
        assert!(matches!(app.overlays.overlay, Overlay::None));
    }

    #[test]
    fn slash_help_sets_help_overlay() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;

        use crate::agent::AgentTurn;
        use crate::slash_exec::{SlashEnv, SlashHandled, handle_slash_line};
        use crate::tui_project::ProjectUiJob;
        use tokio::sync::mpsc;

        let c = sample_config();
        let mut app = TuiApp::new(c.clone());
        let shared = Arc::new(Mutex::new(c));
        app.attach_runtime_handles(Arc::clone(&shared), Arc::new(Notify::new()));
        let (project_tx, _rx) = mpsc::unbounded_channel::<ProjectUiJob>();
        let (agent_tx, _arx) = mpsc::unbounded_channel::<AgentTurn>();
        let env = SlashEnv {
            shared_config: Arc::clone(&shared),
            reload_notify: app.reload_notify.clone().expect("notify"),
            index_enabled_flag: false,
            index_bin_path: app.project_root.join(".akmon").join("index.bin"),
            project_job_tx: project_tx,
            agent_task_tx: agent_tx,
        };
        assert_eq!(
            handle_slash_line(&mut app, "/help", &env),
            Some(SlashHandled::Continue)
        );
        assert!(matches!(app.overlays.overlay, Overlay::Help));
    }

    #[test]
    fn slash_clear_clears_messages() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;

        use crate::agent::AgentTurn;
        use crate::slash_exec::{SlashEnv, SlashHandled, handle_slash_line};
        use crate::tui_project::ProjectUiJob;
        use tokio::sync::mpsc;

        let c = sample_config();
        let mut app = TuiApp::new(c.clone());
        app.messages.clear();
        app.push_user("hi".into());
        let shared = Arc::new(Mutex::new(c));
        app.attach_runtime_handles(Arc::clone(&shared), Arc::new(Notify::new()));
        let (project_tx, _rx) = mpsc::unbounded_channel::<ProjectUiJob>();
        let (agent_tx, _arx) = mpsc::unbounded_channel::<AgentTurn>();
        let env = SlashEnv {
            shared_config: shared.clone(),
            reload_notify: app.reload_notify.clone().expect("notify"),
            index_enabled_flag: false,
            index_bin_path: app.project_root.join(".akmon").join("index.bin"),
            project_job_tx: project_tx,
            agent_task_tx: agent_tx,
        };
        assert_eq!(
            handle_slash_line(&mut app, "/clear", &env),
            Some(SlashHandled::Continue)
        );
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(
            &app.messages[0],
            TuiMessage::SystemInfo { content } if content.contains("on-screen history")
        ));
    }

    #[test]
    fn slash_exit_requests_quit() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;

        use crate::agent::AgentTurn;
        use crate::slash_exec::{SlashEnv, SlashHandled, handle_slash_line};
        use crate::tui_project::ProjectUiJob;
        use tokio::sync::mpsc;

        let c = sample_config();
        let mut app = TuiApp::new(c.clone());
        let shared = Arc::new(Mutex::new(c));
        app.attach_runtime_handles(Arc::clone(&shared), Arc::new(Notify::new()));
        let (project_tx, _rx) = mpsc::unbounded_channel::<ProjectUiJob>();
        let (agent_tx, _arx) = mpsc::unbounded_channel::<AgentTurn>();
        let env = SlashEnv {
            shared_config: shared,
            reload_notify: app.reload_notify.clone().expect("notify"),
            index_enabled_flag: false,
            index_bin_path: app.project_root.join(".akmon").join("index.bin"),
            project_job_tx: project_tx,
            agent_task_tx: agent_tx,
        };
        assert_eq!(
            handle_slash_line(&mut app, "/exit", &env),
            Some(SlashHandled::Quit)
        );
    }

    #[test]
    fn auto_scroll_disables_on_scroll_up() {
        let mut app = TuiApp::new(sample_config());
        app.push_user("a".into());
        app.push_user("b".into());
        let w = 40u16;
        let vh = 2usize;
        app.sync_scroll_to_bottom(vh, w);
        app.scroll_up(10, vh, w);
        if app.total_message_lines(w) > vh {
            assert!(!app.auto_scroll);
        }
    }

    #[test]
    fn auto_scroll_reenables_at_bottom() {
        let mut app = TuiApp::new(sample_config());
        app.push_user("line".into());
        let w = 40u16;
        let vh = 1usize;
        app.scroll_up(50, vh, w);
        app.auto_scroll = false;
        app.scroll_down(10_000, vh, w);
        assert!(app.auto_scroll);
    }

    #[test]
    fn input_insert_backspace_ascii() {
        let mut app = TuiApp::new(sample_config());
        assert!(app.input_insert('h'));
        assert!(app.input_insert('i'));
        assert_eq!(app.composer.buffer, "hi");
        app.input_backspace();
        assert_eq!(app.composer.buffer, "h");
    }

    #[test]
    fn input_left_right_moves_caret_utf8() {
        let mut app = TuiApp::new(sample_config());
        app.composer.buffer = "aβc".into();
        app.composer.cursor = 3;
        app.input_cursor_left();
        assert_eq!(app.composer.cursor, 1);
        app.input_cursor_right();
        assert_eq!(app.composer.cursor, 3);
        app.composer.cursor = 0;
        app.input_cursor_left();
        assert_eq!(app.composer.cursor, 0);
        app.composer.cursor = app.composer.buffer.len();
        app.input_cursor_right();
        assert_eq!(app.composer.cursor, app.composer.buffer.len());
    }

    #[test]
    fn input_allows_many_newlines_for_large_prompts() {
        let mut app = TuiApp::new(sample_config());
        for _ in 0..50 {
            assert!(app.input_insert('\n'));
        }
        assert!(app.composer.buffer.matches('\n').count() >= 49);
    }

    #[test]
    fn input_paste_inserts_at_caret() {
        let mut app = TuiApp::new(sample_config());
        assert!(app.input_insert('a'));
        assert!(app.input_insert('b'));
        app.composer.cursor = 1;
        app.input_paste("XYZ");
        assert_eq!(app.composer.buffer, "aXYZb");
    }

    #[test]
    fn text_delta_appends_open_assistant() {
        let mut app = TuiApp::new(sample_config());
        app.apply_agent_event(AgentEvent::TextDelta { text: "hi".into() });
        app.apply_agent_event(AgentEvent::TextDelta {
            text: " there".into(),
        });
        match app.messages.last() {
            Some(TuiMessage::Assistant { content, complete }) => {
                assert_eq!(content, "hi there");
                assert!(!complete);
            }
            _ => panic!("expected assistant"),
        }
    }

    #[test]
    fn text_delta_new_message_after_complete() {
        let mut app = TuiApp::new(sample_config());
        app.messages.push(TuiMessage::Assistant {
            content: "done".into(),
            complete: true,
        });
        app.apply_agent_event(AgentEvent::TextDelta {
            text: "next".into(),
        });
        assert!(matches!(
            app.messages.last(),
            Some(TuiMessage::Assistant { content, complete: false }) if content == "next"
        ));
    }

    #[test]
    fn tool_dispatched_then_completed() {
        let mut app = TuiApp::new(sample_config());
        app.apply_agent_event(AgentEvent::ToolCallDispatched {
            id: "t1".into(),
            name: "list_directory".into(),
            arguments: json!({"path": "."}),
        });
        app.apply_agent_event(AgentEvent::ToolCallCompleted {
            id: "t1".into(),
            name: "list_directory".into(),
            success: true,
            message: "ok".into(),
        });
        match app.messages.last() {
            Some(TuiMessage::ToolCall {
                result, success, ..
            }) => {
                assert_eq!(result.as_deref(), Some("ok"));
                assert_eq!(*success, Some(true));
            }
            _ => panic!("expected tool"),
        }
    }

    #[test]
    fn confirmation_sets_awaiting() {
        let mut app = TuiApp::new(sample_config());
        assert!(!app.overlays.awaiting_confirmation);
        app.apply_agent_event(AgentEvent::ConfirmationRequired {
            description: "allow?".into(),
            diff_preview: None,
        });
        assert!(app.overlays.awaiting_confirmation);
    }

    #[test]
    fn usage_accumulates() {
        let mut app = TuiApp::new(sample_config());
        app.apply_agent_event(AgentEvent::UsageReport {
            input_tokens: 10,
            output_tokens: 3,
            cache_creation_tokens: 2,
            cache_read_tokens: 5,
        });
        assert_eq!(app.telemetry.total_input_tokens, 10);
        assert_eq!(app.telemetry.total_output_tokens, 3);
        assert_eq!(app.telemetry.total_cache_read_tokens, 5);
        assert_eq!(app.telemetry.total_cache_write_tokens, 2);
    }

    #[test]
    fn submit_user_message_preserves_behavior() {
        let mut app = TuiApp::new(sample_config());
        app.composer.buffer = "  hello world  ".into();
        app.composer.cursor = app.composer.buffer.len();
        let out = app.submit_user_message();
        assert_eq!(out.as_deref(), Some("hello world"));
        assert_eq!(app.telemetry.message_count, 1);
        assert_eq!(app.composer.cursor, 0);
        assert!(app.composer.buffer.is_empty());
    }

    #[test]
    fn stream_cursor_tick_preserves_toggle_behavior() {
        let mut app = TuiApp::new(sample_config());
        let before = app.runtime.stream_cursor_visible;
        app.tick_stream_cursor();
        assert_ne!(before, app.runtime.stream_cursor_visible);
    }

    #[test]
    fn mark_confirmation_answered() {
        let mut app = TuiApp::new(sample_config());
        app.apply_agent_event(AgentEvent::ConfirmationRequired {
            description: "x".into(),
            diff_preview: None,
        });
        app.mark_confirmation_answered(true);
        assert!(!app.overlays.awaiting_confirmation);
        match app.messages.last() {
            Some(TuiMessage::Confirmation {
                answered, answer, ..
            }) => {
                assert!(*answered);
                assert_eq!(*answer, Some(true));
            }
            _ => panic!("expected confirmation"),
        }
    }

    #[test]
    fn note_touched_file_dedupes_in_order() {
        let mut app = TuiApp::new(sample_config());
        app.note_touched_file("src/a.rs");
        app.note_touched_file("src/a.rs");
        app.note_touched_file("src/b.rs");
        assert_eq!(
            app.telemetry.session_touched_files,
            vec!["src/a.rs", "src/b.rs"]
        );
    }
}
