//! Mutable TUI state: transcript, input, scrolling, and usage counters.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ratatui::layout::Rect;

use akmon_core::AgentEvent;
use chrono::{DateTime, Utc};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::command::UiCommand;
use crate::config::TuiLaunchConfig;
use crate::message::TuiMessage;
use crate::session_persist::SessionSummary;
use crate::slash::SlashCommand;

/// One line in [`Overlay::ModelPicker`]: either a section title or a selectable model id.
#[derive(Debug, Clone)]
pub struct ModelPickerRow {
    /// When `true`, `label` is a section heading (not selectable).
    pub section_header: bool,
    /// Section title or model id.
    pub label: String,
}

/// Modal overlay drawn over the transcript or above the input (slash UI).
#[derive(Debug)]
pub enum Overlay {
    /// No overlay; normal chat view.
    None,
    /// `/help` — lists slash commands (any key closes).
    Help,
    /// `/sessions` or bare `/resume` — pick a saved session.
    SessionList {
        /// Newest-first rows from `~/.akmon/sessions/`.
        sessions: Vec<SessionSummary>,
        /// Highlighted row index.
        selected: usize,
        /// First visible row when the list scrolls.
        scroll: usize,
    },
    /// `/audit` — JSONL audit lines for the active session.
    AuditLog {
        /// Pre-formatted rows (`timestamp kind description`).
        lines: Vec<String>,
        /// Index of the first visible line.
        scroll: usize,
    },
    /// `/cost` — token table and cost hint (any key closes).
    CostSummary,
    /// `/model` with no argument — pick a model from configured providers.
    ModelPicker {
        /// All rows (headers + models).
        rows: Vec<ModelPickerRow>,
        /// Indices into `rows` for selectable model lines.
        selectable: Vec<usize>,
        /// Index into `selectable`.
        selected: usize,
        /// First row index shown when the list scrolls.
        scroll: usize,
    },
    /// Command-name completion while the input starts with `/`.
    SlashAutocomplete {
        /// Filtered commands (at most six visible; rest scroll).
        matches: Vec<&'static SlashCommand>,
        /// Highlighted match index.
        selected: usize,
    },
}

/// Primary application state for the interactive terminal UI.
pub struct TuiApp {
    /// Transcript rows (user, assistant, tools, …).
    pub messages: Vec<TuiMessage>,
    /// Current input draft (may contain newlines).
    pub input_buffer: String,
    /// Byte index of the caret inside [`Self::input_buffer`].
    pub input_cursor: usize,
    /// First visible line index inside the flattened message list.
    pub scroll_offset: usize,
    /// When `true`, new content keeps the view pinned to the bottom.
    pub auto_scroll: bool,
    /// Session identifier (shown truncated in the status bar).
    pub session_id: Uuid,
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
    /// Whether an agent turn is in flight.
    pub agent_running: bool,
    /// Latest iteration index reported by the agent (from [`AgentEvent::IterationStarted`]).
    pub current_iteration: u32,
    /// Maximum agent iterations (updated from events when present).
    pub max_iterations: u32,
    /// Cumulative input tokens reported by the provider.
    pub total_input_tokens: u32,
    /// Cumulative prompt-cache read tokens (cache hits).
    pub total_cache_read_tokens: u32,
    /// Cumulative output tokens.
    pub total_output_tokens: u32,
    /// Toggles streaming cursor visibility on a fixed interval.
    pub stream_cursor_visible: bool,
    /// `--index` flag echo for the header.
    pub index_enabled: bool,
    /// When `true`, only confirmation keys are accepted in the input handler.
    pub awaiting_confirmation: bool,
    /// Channel to the agent task (confirm / interrupt).
    pub ui_command_tx: Option<UnboundedSender<UiCommand>>,
    /// Wall-clock start for session snapshot metadata.
    pub session_started_at: DateTime<Utc>,
    /// Active full-screen or picker overlay (slash-driven).
    pub overlay: Overlay,
    /// Cumulative prompt-cache **write** (creation) tokens from usage reports.
    pub total_cache_write_tokens: u32,
    /// Resolved audit JSONL path for this session (updated on `/reset` and `/resume`).
    pub audit_log_path: PathBuf,
    /// Selected row in [`Overlay::SlashAutocomplete`]; persisted while the prefix is stable.
    pub slash_ac_selected: usize,
    /// Fingerprint of the slash prefix + first match label to reset selection when filtering changes.
    pub slash_ac_sig: String,
    /// After Esc on autocomplete, hide the menu until the user edits the buffer again.
    pub slash_ac_suppress: bool,
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
    /// Next message uses read-only plan mode (`/plan`).
    pub plan_only_next_turn: bool,
    /// Next message runs architect (planner + main model).
    pub architect_next_turn: bool,
    /// Last plan output for `/implement`.
    pub pending_plan: Option<String>,
}

impl TuiApp {
    /// Builds initial state from launch metadata.
    pub fn new(config: TuiLaunchConfig) -> Self {
        let session_id = config.session_id;
        let started = Utc::now();
        let project_name = config
            .project_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(".")
            .to_string();
        Self {
            messages: Vec::new(),
            input_buffer: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            auto_scroll: true,
            session_id,
            project_root: config.project_root,
            project_name,
            model_name: config.model_name,
            mode_label: config.mode_label,
            version: config.version,
            agent_running: false,
            current_iteration: 0,
            max_iterations: config.max_iterations,
            total_input_tokens: 0,
            total_cache_read_tokens: 0,
            total_output_tokens: 0,
            stream_cursor_visible: true,
            index_enabled: config.index_enabled,
            awaiting_confirmation: false,
            ui_command_tx: None,
            session_started_at: started,
            overlay: Overlay::None,
            total_cache_write_tokens: 0,
            audit_log_path: config.audit_log_path.clone(),
            slash_ac_selected: 0,
            slash_ac_sig: String::new(),
            slash_ac_suppress: false,
            shared_launch_config: None,
            reload_notify: None,
            input_body_inner: None,
            welcome_spark_phase: false,
            has_akmon_md: config.has_akmon_md,
            plan_only_next_turn: false,
            architect_next_turn: false,
            pending_plan: None,
        }
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
        if self.input_buffer.trim().is_empty() {
            return None;
        }
        let raw = std::mem::take(&mut self.input_buffer);
        self.input_cursor = 0;
        let t = raw.trim().to_string();
        self.push_user(t.clone());
        Some(t)
    }

    /// Applies one streamed [`AgentEvent`] to the transcript and counters.
    pub fn apply_agent_event(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::TextDelta { text } => {
                if text.is_empty() {
                    return;
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
                success,
                message,
                ..
            } => {
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
            AgentEvent::ConfirmationRequired { description } => {
                self.awaiting_confirmation = true;
                self.messages.push(TuiMessage::Confirmation {
                    description,
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
                self.total_input_tokens = self.total_input_tokens.saturating_add(input_tokens);
                self.total_output_tokens = self.total_output_tokens.saturating_add(output_tokens);
                self.total_cache_read_tokens = self
                    .total_cache_read_tokens
                    .saturating_add(cache_read_tokens);
                self.total_cache_write_tokens = self
                    .total_cache_write_tokens
                    .saturating_add(cache_creation_tokens);
            }
            AgentEvent::IterationStarted { n, max } => {
                self.current_iteration = n;
                self.max_iterations = max;
            }
            AgentEvent::Done => {
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
        self.awaiting_confirmation = false;
        if let Some(TuiMessage::Confirmation {
            answered,
            answer,
            ..
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
            .map(|m| crate::render::message_line_count(m, width))
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
        self.slash_ac_suppress = false;
        const MAX_INPUT_CHARS: usize = 16_384;
        if self.input_buffer.len() >= MAX_INPUT_CHARS {
            return false;
        }
        if ch == '\n' {
            let lines = self.input_buffer.matches('\n').count() + 1;
            if lines >= 6 {
                return false;
            }
        }
        let idx = self.input_cursor.min(self.input_buffer.len());
        self.input_buffer.insert(idx, ch);
        self.input_cursor = self.input_cursor.saturating_add(ch.len_utf8());
        true
    }

    /// Removes the grapheme before the caret (ASCII slice-1 for slice 1).
    pub fn input_backspace(&mut self) {
        self.slash_ac_suppress = false;
        if self.input_cursor == 0 {
            return;
        }
        let idx = self.input_cursor.saturating_sub(1);
        if self.input_buffer.is_char_boundary(idx) {
            self.input_buffer.remove(idx);
            self.input_cursor = idx;
        }
    }

    /// Deletes the character under the caret when present.
    pub fn input_delete_forward(&mut self) {
        self.slash_ac_suppress = false;
        if self.input_cursor >= self.input_buffer.len() {
            return;
        }
        if self.input_buffer.is_char_boundary(self.input_cursor) {
            self.input_buffer.remove(self.input_cursor);
        }
    }

    /// Moves the caret one Unicode scalar left (no-op at start).
    pub fn input_cursor_left(&mut self) {
        self.slash_ac_suppress = false;
        if self.input_cursor == 0 {
            return;
        }
        let slice = &self.input_buffer[..self.input_cursor];
        let prev = slice
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input_cursor = prev;
    }

    /// Moves the caret one Unicode scalar right (no-op at end).
    pub fn input_cursor_right(&mut self) {
        self.slash_ac_suppress = false;
        if self.input_cursor >= self.input_buffer.len() {
            return;
        }
        let slice = &self.input_buffer[self.input_cursor..];
        let mut it = slice.chars();
        if let Some(c) = it.next() {
            self.input_cursor += c.len_utf8();
        }
    }

    /// Syncs scroll offset to the bottom (e.g. after resize when auto-scroll is on).
    pub fn sync_scroll_to_bottom(&mut self, viewport_height: usize, width: u16) {
        if self.auto_scroll {
            self.scroll_offset = self.max_scroll_offset(viewport_height, width);
        }
    }

    /// Flips the streaming cursor blink bit (call on a timer tick).
    pub fn tick_stream_cursor(&mut self) {
        self.stream_cursor_visible = !self.stream_cursor_visible;
    }

    /// After appending transcript rows, pin the viewport to the newest line when [`Self::auto_scroll`] is enabled.
    pub fn recompute_scroll_after_append(&mut self, viewport_height: usize, width: u16) {
        if self.auto_scroll {
            self.scroll_offset = self.max_scroll_offset(viewport_height, width);
        }
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
        }
    }

    #[test]
    fn new_defaults() {
        let app = TuiApp::new(sample_config());
        assert!(app.auto_scroll);
        assert!(!app.agent_running);
        assert_eq!(app.input_cursor, 0);
        assert!(app.input_buffer.is_empty());
        assert_eq!(app.max_iterations, 25);
        assert!(app.messages.is_empty());
        assert!(matches!(app.overlay, Overlay::None));
    }

    #[test]
    fn slash_help_sets_help_overlay() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;

        use crate::agent::AgentTurn;
        use crate::slash_exec::{handle_slash_line, SlashEnv, SlashHandled};
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
        assert!(matches!(app.overlay, Overlay::Help));
    }

    #[test]
    fn slash_clear_clears_messages() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;

        use crate::agent::AgentTurn;
        use crate::slash_exec::{handle_slash_line, SlashEnv, SlashHandled};
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
            TuiMessage::SystemInfo { content } if content.contains("Session cleared")
        ));
    }

    #[test]
    fn slash_exit_requests_quit() {
        use std::sync::{Arc, Mutex};
        use tokio::sync::Notify;

        use crate::agent::AgentTurn;
        use crate::slash_exec::{handle_slash_line, SlashEnv, SlashHandled};
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
        assert_eq!(app.input_buffer, "hi");
        app.input_backspace();
        assert_eq!(app.input_buffer, "h");
    }

    #[test]
    fn input_left_right_moves_caret_utf8() {
        let mut app = TuiApp::new(sample_config());
        app.input_buffer = "aβc".into();
        app.input_cursor = 3;
        app.input_cursor_left();
        assert_eq!(app.input_cursor, 1);
        app.input_cursor_right();
        assert_eq!(app.input_cursor, 3);
        app.input_cursor = 0;
        app.input_cursor_left();
        assert_eq!(app.input_cursor, 0);
        app.input_cursor = app.input_buffer.len();
        app.input_cursor_right();
        assert_eq!(app.input_cursor, app.input_buffer.len());
    }

    #[test]
    fn input_newline_cap_six_lines() {
        let mut app = TuiApp::new(sample_config());
        for _ in 0..5 {
            assert!(app.input_insert('\n'));
        }
        assert!(!app.input_insert('\n'));
    }

    #[test]
    fn text_delta_appends_open_assistant() {
        let mut app = TuiApp::new(sample_config());
        app.apply_agent_event(AgentEvent::TextDelta {
            text: "hi".into(),
        });
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
                result,
                success,
                ..
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
        assert!(!app.awaiting_confirmation);
        app.apply_agent_event(AgentEvent::ConfirmationRequired {
            description: "allow?".into(),
        });
        assert!(app.awaiting_confirmation);
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
        assert_eq!(app.total_input_tokens, 10);
        assert_eq!(app.total_output_tokens, 3);
        assert_eq!(app.total_cache_read_tokens, 5);
        assert_eq!(app.total_cache_write_tokens, 2);
    }

    #[test]
    fn mark_confirmation_answered() {
        let mut app = TuiApp::new(sample_config());
        app.apply_agent_event(AgentEvent::ConfirmationRequired {
            description: "x".into(),
        });
        app.mark_confirmation_answered(true);
        assert!(!app.awaiting_confirmation);
        match app.messages.last() {
            Some(TuiMessage::Confirmation {
                answered,
                answer,
                ..
            }) => {
                assert!(*answered);
                assert_eq!(*answer, Some(true));
            }
            _ => panic!("expected confirmation"),
        }
    }
}
