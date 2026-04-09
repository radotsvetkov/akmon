//! TUI rendering: transcript flattening, chrome, and overlays.

mod code;
mod dialog_overlay;
mod flatten;
mod header_bar;
mod input_hit;
mod status_bar;
mod viewport_paint;
mod wrap;

pub use dialog_overlay::{
    dialog_from_confirmation, render_confirmation_overlay, render_question_overlay,
    shell_prefix_hint,
};
pub use flatten::{flatten_transcript, message_line_count, message_to_lines};
pub use header_bar::render_header_bar;
pub use input_hit::{input_body_row_count, map_input_click_wrapped, snap_utf8_cursor};
pub use status_bar::{
    CostFrag, StatusParts, context_usage_percent, context_window_for_model, render_context_bar,
    render_status_bar,
};
pub use viewport_paint::{paint_message_viewport, paint_terminal_too_small};
