//! Ratatui-based interactive terminal UI for Akmon (v1.3+).
//!
//! The agent loop runs on the Tokio runtime; the terminal uses `spawn_blocking` and exchanges
//! state through a `std::sync::mpsc` bridge.

#![warn(missing_docs)]

mod agent;
mod app;
mod command;
mod config;
mod cost_estimate;
mod layout;
mod message;
mod model_picker;
mod overlay;
mod paths;
mod render;
mod runner;
mod session_persist;
mod slash;
mod slash_exec;
mod state;
mod theme;
mod tui_project;
mod welcome;

pub use agent::{AgentTurn, BridgeMsg};
pub use app::{ExternalEditTarget, TuiApp};
pub use command::UiCommand;
#[cfg(feature = "semantic-index")]
pub use config::SemanticIndexSlot;
pub use config::TuiLaunchConfig;
pub use cost_estimate::estimate_cost_usd;
pub use message::TuiMessage;
pub use render::{message_line_count, message_to_lines, paint_message_viewport};
pub use runner::{TuiRunError, run_blocking, run_interactive};
pub use session_persist::{
    LoadedSession, SessionSummary, default_audit_log_path, latest_dot_akmon_plan,
    load_session_file, load_session_summaries, save_session_snapshot,
    saved_sessions_dir_has_no_json, saved_sessions_directory_empty, session_file_path_for,
    sessions_directory, sessions_directory_under_home,
};
pub use slash::{COMMANDS, SlashCommand, matching_commands, parse_slash_input};
pub use slash_exec::{SlashEnv, SlashHandled, handle_slash_line};
