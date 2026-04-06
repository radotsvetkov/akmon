//! Ratatui-based interactive terminal UI for Akmon (v1.3+).
//!
//! The agent loop runs on the Tokio runtime; the terminal uses `spawn_blocking` and exchanges
//! state through a `std::sync::mpsc` bridge.

#![warn(missing_docs)]

mod agent;
mod app;
mod command;
mod config;
mod message;
mod model_picker;
mod overlay;
mod render;
mod runner;
mod session_persist;
mod slash;
mod slash_exec;
mod theme;
mod tui_project;
mod welcome;

pub use agent::{AgentTurn, BridgeMsg};
pub use app::TuiApp;
pub use command::UiCommand;
#[cfg(feature = "semantic-index")]
pub use config::SemanticIndexSlot;
pub use config::TuiLaunchConfig;
pub use message::TuiMessage;
pub use render::{message_line_count, message_to_lines, paint_message_viewport};
pub use runner::{run_blocking, run_interactive, TuiRunError};
pub use session_persist::{
    default_audit_log_path, load_session_file, load_session_summaries, save_session_snapshot,
    session_file_path_for, sessions_directory, sessions_directory_under_home, LoadedSession,
    SessionSummary,
};
pub use slash::{matching_commands, parse_slash_input, SlashCommand, COMMANDS};
pub use slash_exec::{handle_slash_line, SlashEnv, SlashHandled};
