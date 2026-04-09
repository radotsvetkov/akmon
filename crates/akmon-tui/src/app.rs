//! Mutable TUI state: transcript, input, scrolling, and usage counters.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ratatui::layout::Rect;

use akmon_core::{AgentEvent, ContextScan, scan_context_files};
use akmon_models::OllamaProbe;
use chrono::{DateTime, Utc};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::command::{SessionSideEffect, UiCommand};
use crate::config::TuiLaunchConfig;
use crate::layout::LayoutRects;
use crate::message::TuiMessage;
use crate::render::context_usage_percent;
use crate::session_persist::SessionSummary;
use crate::slash::SlashCommand;
use crate::state::{AgentDisplayState, ConfirmationDialog, SettingsOverlayState};

/// Target file to open in the user's `EDITOR` outside the alternate-screen TUI.
#[derive(Debug, Clone)]
pub enum ExternalEditTarget {
    /// Latest saved implementation plan under `.akmon/plans/`.
    Plan(std::path::PathBuf),
    /// Project `AKMON.md`.
    AkmonMd(std::path::PathBuf),
}

/// One line in [`Overlay::ModelPicker`]: either a section title or a selectable model id.
#[derive(Debug, Clone)]
pub struct ModelPickerRow {
    /// When `true`, `label` is a section heading (not selectable).
    pub section_header: bool,
    /// When `false`, the row is a note (e.g. “not configured”) and cannot be selected.
    pub selectable: bool,
    /// Model id used when applying selection (ignored for headers / notes).
    pub label: String,
    /// Optional override for on-screen text (e.g. bullet + size column).
    pub display: Option<String>,
}

/// Modal overlay drawn over the transcript or above the input (slash UI).
#[derive(Debug)]
pub enum Overlay {
    /// No overlay; normal chat view.
    None,
    /// `/help` — lists slash commands (any key closes).
    Help,
    /// `/sessions` — picker for saved snapshots under `~/.akmon/sessions/`.
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
    /// `/view-plan`, `/mcp` — scrollable plain-text overlay (full width).
    ScrollText {
        /// Title shown in the border (e.g. plan path or "MCP servers").
        title: String,
        /// One string per source line; long lines wrap within the viewport.
        lines: Vec<String>,
        /// Index of the first visible source line.
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
    /// `/config` or Ctrl+S — tabs for estimates and pointer text for other panels.
    Settings(SettingsOverlayState),
}

/// State for an `ask_followup` prompt (user types the reply in the compose area).
#[derive(Debug, Clone)]
pub struct QuestionPromptState {
    /// Tool call id matching the transcript row.
    pub call_id: String,
    /// Question body from the model.
    pub question: String,
    /// Optional quick replies.
    pub suggestions: Vec<String>,
}

/// Primary application state for the interactive terminal UI.
pub struct TuiApp {
    /// Last known full-terminal bounds (updated each frame and on resize).
    pub terminal_size: Rect,
    /// Cached root layout from [`crate::layout::compute_layout`].
    pub layout_rects: LayoutRects,
    /// Centered permission dialog state while [`Self::awaiting_confirmation`].
    pub confirmation_dialog: Option<ConfirmationDialog>,
    /// Short status-bar hint after Ctrl+C on an empty buffer (cleared on next input).
    pub status_flash: Option<String>,
    /// High-level agent UI phase (spinner / streaming / …).
    pub agent_display: AgentDisplayState,
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
    /// Cached [`TuiLaunchConfig::provider_display_name`] for the status bar / exit summary.
    pub provider_display_name: String,
    /// Routing via OpenRouter (`model` contains `/`).
    pub uses_openrouter: bool,
    /// Ollama fallback (no billable cloud keys in use).
    pub free_local_inference: bool,
    /// Use terminal default foreground for transcript body (see `[display] theme = "light"`).
    pub light_body_text: bool,
    /// Whether an agent turn is in flight.
    pub agent_running: bool,
    /// Short status for the header when [`Self::agent_running`] (e.g. “Model is responding…”).
    pub agent_activity_line: String,
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
    /// Whether the 80% context warning has already been shown this session.
    pub context_warn_80_shown: bool,
    /// Whether the 90% context warning has already been shown this session.
    pub context_warn_90_shown: bool,
    /// Toggles streaming cursor visibility on a fixed interval.
    pub stream_cursor_visible: bool,
    /// `--index` flag echo for the header.
    pub index_enabled: bool,
    /// When `true`, only confirmation keys are accepted in the input handler.
    pub awaiting_confirmation: bool,
    /// When `true`, the user is answering an `ask_followup` prompt.
    pub awaiting_question: bool,
    /// Pending question UI state.
    pub question_prompt: Option<QuestionPromptState>,
    /// Channel to the agent task (confirm / interrupt).
    pub ui_command_tx: Option<UnboundedSender<UiCommand>>,
    /// Slash `/clear` and similar session maintenance handled on the agent task.
    pub session_effect_tx: Option<UnboundedSender<SessionSideEffect>>,
    /// Wall-clock start for session snapshot metadata.
    pub session_started_at: DateTime<Utc>,
    /// Active full-screen or picker overlay (slash-driven).
    pub overlay: Overlay,
    /// Cumulative prompt-cache **write** (creation) tokens from usage reports.
    pub total_cache_write_tokens: u32,
    /// Rough input tokens reclaimed by clearing stale tool outputs (see [`AgentEvent::MicrocompactEstimate`]).
    pub total_microcompact_cleared: u32,
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
    /// User messages submitted this session (exit summary).
    pub message_count: u32,
    /// Tool invocations finished this session.
    pub total_tool_calls: u32,
    /// Tool runs that reported success (exit summary).
    pub successful_tool_calls: u32,
    /// Tool runs that reported failure (exit summary).
    pub failed_tool_calls: u32,
    /// Distinct files read successfully (`read_file`).
    pub files_read: Vec<String>,
    /// Distinct files written or edited successfully.
    pub files_written: Vec<String>,
    /// Union of read/write/edit paths for the context bar (deduplicated, recent-first awareness via order).
    pub session_touched_files: Vec<String>,
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
    /// Latest Ollama `/api/tags` probe (background refresh + `/model` / `/doctor`).
    pub ollama_probe: OllamaProbe,
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
            confirmation_dialog: None,
            status_flash: None,
            agent_display: AgentDisplayState::Idle,
            messages,
            input_buffer: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            auto_scroll: true,
            session_id,
            session_display_name,
            project_root: config.project_root,
            project_name,
            model_name: config.model_name,
            mode_label: config.mode_label,
            version: config.version,
            provider_display_name,
            uses_openrouter,
            free_local_inference,
            light_body_text,
            agent_running: false,
            agent_activity_line: String::new(),
            current_iteration: 0,
            max_iterations: config.max_iterations,
            total_input_tokens: 0,
            total_cache_read_tokens: 0,
            total_output_tokens: 0,
            context_warn_80_shown: false,
            context_warn_90_shown: false,
            stream_cursor_visible: true,
            index_enabled: config.index_enabled,
            awaiting_confirmation: false,
            awaiting_question: false,
            question_prompt: None,
            ui_command_tx: None,
            session_effect_tx: None,
            session_started_at: started,
            overlay: Overlay::None,
            total_cache_write_tokens: 0,
            total_microcompact_cleared: 0,
            audit_log_path: config.audit_log_path.clone(),
            slash_ac_selected: 0,
            slash_ac_sig: String::new(),
            slash_ac_suppress: false,
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
            message_count: 0,
            total_tool_calls: 0,
            successful_tool_calls: 0,
            failed_tool_calls: 0,
            files_read: Vec::new(),
            files_written: Vec::new(),
            session_touched_files: Vec::new(),
            model_estimates: config.model_estimates.clone(),
            resume_pin_bottom: false,
            pending_external_edit: None,
            spinner_frame: 0,
            // Default ON so the wheel reaches the full transcript; Shift+drag still selects text in many terminals.
            // Ctrl+M toggles if you prefer native selection without wheel routing.
            mouse_capture_enabled: true,
            mouse_capture_applied: false,
            ollama_probe: OllamaProbe {
                reachable: false,
                models: vec![],
            },
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
        if self.input_buffer.trim().is_empty() {
            return None;
        }
        let raw = std::mem::take(&mut self.input_buffer);
        self.input_cursor = 0;
        let t = raw.trim().to_string();
        self.push_user(t.clone());
        self.has_sent_first_message = true;
        self.message_count = self.message_count.saturating_add(1);
        Some(t)
    }

    fn push_unique(list: &mut Vec<String>, path: String) {
        if path.is_empty() || list.iter().any(|p| p == &path) {
            return;
        }
        list.push(path);
    }

    /// Records a sandbox-relative path in [`Self::session_touched_files`] for the status context bar.
    pub fn note_touched_file(&mut self, path: &str) {
        Self::push_unique(&mut self.session_touched_files, path.to_string());
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
                if self.agent_running {
                    self.agent_activity_line = "Model is responding…".into();
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
                self.agent_activity_line = match name.as_str() {
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
                self.agent_activity_line = if success {
                    "Tool finished — continuing…"
                } else {
                    "Tool failed — model may adjust…"
                }
                .into();
                self.total_tool_calls = self.total_tool_calls.saturating_add(1);
                if success {
                    self.successful_tool_calls = self.successful_tool_calls.saturating_add(1);
                    if let Some(path) = Self::tool_path_from_messages(&self.messages, &id) {
                        match name.as_str() {
                            "read_file" => Self::push_unique(&mut self.files_read, path.clone()),
                            "write_file" | "edit" => {
                                Self::push_unique(&mut self.files_written, path.clone());
                            }
                            _ => {}
                        }
                        self.note_touched_file(&path);
                    }
                } else {
                    self.failed_tool_calls = self.failed_tool_calls.saturating_add(1);
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
                self.agent_activity_line =
                    "Answer the question below — Enter to submit, Esc to skip".into();
                self.awaiting_question = true;
                self.input_buffer.clear();
                self.input_cursor = 0;
                self.question_prompt = Some(QuestionPromptState {
                    call_id: id,
                    question,
                    suggestions,
                });
            }
            AgentEvent::ConfirmationRequired {
                description,
                diff_preview,
            } => {
                self.agent_activity_line =
                    "Waiting for your approval — choose an option below".into();
                self.awaiting_confirmation = true;
                self.confirmation_dialog = Some(crate::render::dialog_from_confirmation(
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
                    self.agent_activity_line = "continuing response…".into();
                } else if message.starts_with('⟳') {
                    self.agent_activity_line = message.clone();
                }
            }
            AgentEvent::MicrocompactEstimate {
                estimated_tokens_cleared,
            } => {
                self.total_microcompact_cleared = self
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
                self.total_input_tokens = self.total_input_tokens.saturating_add(input_tokens);
                self.total_output_tokens = self.total_output_tokens.saturating_add(output_tokens);
                self.total_cache_read_tokens = self
                    .total_cache_read_tokens
                    .saturating_add(cache_read_tokens);
                self.total_cache_write_tokens = self
                    .total_cache_write_tokens
                    .saturating_add(cache_creation_tokens);
                let pct = context_usage_percent(
                    self.total_input_tokens,
                    self.total_cache_read_tokens,
                    &self.model_name,
                    &self.model_estimates,
                );
                if pct >= 90 && !self.context_warn_90_shown {
                    self.status_flash =
                        Some("─ context at 90% — auto-compact will trigger soon ─".into());
                    self.context_warn_90_shown = true;
                    self.context_warn_80_shown = true;
                } else if pct >= 80 && !self.context_warn_80_shown {
                    self.status_flash = Some("─ context at 80% — consider /compact soon ─".into());
                    self.context_warn_80_shown = true;
                }
            }
            AgentEvent::ProviderConfirmed { provider, .. } => {
                self.provider_display_name = provider.clone();
                self.uses_openrouter = provider == "OpenRouter";
                self.free_local_inference = provider == "Ollama";
            }
            AgentEvent::IterationStarted { n, max } => {
                self.current_iteration = n;
                self.max_iterations = max;
                self.agent_activity_line = format!("Step {n}/{max} · contacting model…");
            }
            AgentEvent::Done => {
                // Fallback guard: the runner also flips this on RunFinished, but
                // setting it here keeps input unblocked if Done arrives early.
                self.agent_running = false;
                self.agent_activity_line.clear();
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
        self.confirmation_dialog = None;
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
                    self.stream_cursor_visible,
                    self.light_body_text,
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
        self.slash_ac_suppress = false;
        const MAX_INPUT_BYTES: usize = 512 * 1024;
        if self.input_buffer.len() >= MAX_INPUT_BYTES {
            return false;
        }
        let idx = self.input_cursor.min(self.input_buffer.len());
        self.input_buffer.insert(idx, ch);
        self.input_cursor = self.input_cursor.saturating_add(ch.len_utf8());
        true
    }

    /// Inserts pasted or bulk text at the caret (used for terminal bracketed paste).
    pub fn input_paste(&mut self, text: &str) {
        self.slash_ac_suppress = false;
        const MAX_INPUT_BYTES: usize = 512 * 1024;
        if self.input_buffer.len() >= MAX_INPUT_BYTES {
            return;
        }
        let idx = self.input_cursor.min(self.input_buffer.len());
        let remain = MAX_INPUT_BYTES.saturating_sub(self.input_buffer.len());
        if remain == 0 {
            return;
        }
        let mut end = text.len().min(remain);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        self.input_buffer.insert_str(idx, &text[..end]);
        self.input_cursor += end;
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

    /// Derives [`Self::agent_display`] from flags and the transcript tail.
    pub fn sync_agent_display(&mut self) {
        self.agent_display = if self.awaiting_confirmation {
            AgentDisplayState::WaitingForConfirmation
        } else if !self.agent_running {
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
                step: self.current_iteration,
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
            model_estimates: Vec::new(),
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
        assert!(matches!(app.overlay, Overlay::Help));
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
    fn input_allows_many_newlines_for_large_prompts() {
        let mut app = TuiApp::new(sample_config());
        for _ in 0..50 {
            assert!(app.input_insert('\n'));
        }
        assert!(app.input_buffer.matches('\n').count() >= 49);
    }

    #[test]
    fn input_paste_inserts_at_caret() {
        let mut app = TuiApp::new(sample_config());
        assert!(app.input_insert('a'));
        assert!(app.input_insert('b'));
        app.input_cursor = 1;
        app.input_paste("XYZ");
        assert_eq!(app.input_buffer, "aXYZb");
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
        assert!(!app.awaiting_confirmation);
        app.apply_agent_event(AgentEvent::ConfirmationRequired {
            description: "allow?".into(),
            diff_preview: None,
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
            diff_preview: None,
        });
        app.mark_confirmation_answered(true);
        assert!(!app.awaiting_confirmation);
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
        assert_eq!(app.session_touched_files, vec!["src/a.rs", "src/b.rs"]);
    }
}
