//! Viewport painting: welcome or transcript slice.

use akmon_core::ContextScan;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::welcome::render_welcome;

/// Paints either the branded welcome or the visible transcript lines.
#[allow(clippy::too_many_arguments)]
pub fn paint_message_viewport(
    f: &mut ratatui::Frame<'_>,
    msg_area: Rect,
    show_welcome: bool,
    version: &str,
    project_name: &str,
    welcome_spark_phase: bool,
    first_session_ever: bool,
    has_sent_first_message: bool,
    has_akmon_md: bool,
    context_scan: &ContextScan,
    visible: Vec<Line<'static>>,
) {
    let target = msg_area;
    if show_welcome {
        f.render_widget(Clear, target);
        render_welcome(
            target,
            f.buffer_mut(),
            version,
            project_name,
            welcome_spark_phase,
            first_session_ever,
            has_sent_first_message,
            has_akmon_md,
            context_scan,
        );
    } else {
        f.render_widget(
            Paragraph::new(visible).block(Block::default().borders(Borders::NONE)),
            target,
        );
    }
}

/// "Terminal too small" centered notice.
pub fn paint_terminal_too_small(f: &mut ratatui::Frame<'_>, area: Rect) {
    let msg = "Terminal too small — resize to at least 80×24";
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(msg).alignment(ratatui::layout::Alignment::Center),
        area,
    );
}
