//! Conversation rows rendered in the TUI message pane.

use serde_json::Value;

/// One logical message in the scrollable transcript.
#[derive(Debug, Clone)]
pub enum TuiMessage {
    /// User input line(s).
    User {
        /// Raw text the user submitted.
        content: String,
    },
    /// Assistant reply, optionally still streaming.
    Assistant {
        /// Markdown-ish body (fences and `**bold**` supported in the renderer).
        content: String,
        /// When `false`, the renderer appends a blinking cursor.
        complete: bool,
    },
    /// Tool invocation and outcome (expand/collapse is slice 1 visual only).
    ToolCall {
        /// Stable tool-use id from the model.
        id: String,
        /// Registered tool name.
        name: String,
        /// JSON arguments object.
        args: Value,
        /// Short result text when finished.
        result: Option<String>,
        /// Whether the tool reported success (when finished).
        success: Option<bool>,
        /// When `true`, the card shows JSON args and result.
        expanded: bool,
    },
    /// Policy confirmation prompt (answered in slice 2+).
    Confirmation {
        /// Human-readable action description.
        description: String,
        /// Whether the user has responded.
        answered: bool,
        /// Allow (`true`) or deny (`false`) when answered.
        answer: Option<bool>,
    },
    /// Low-priority informational line.
    SystemInfo {
        /// Free-form status text.
        content: String,
    },
    /// Recoverable or fatal error text.
    Error {
        /// Error description.
        content: String,
    },
}
