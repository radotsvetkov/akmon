//! Single-line header: brand, cwd, model / provider.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::Path;

use crate::paths::cwd_shortened;

const BG: ratatui::style::Color = ratatui::style::Color::Rgb(23, 23, 27);
const RULE: ratatui::style::Color = ratatui::style::Color::Rgb(42, 42, 46);
const AMBER: ratatui::style::Color = ratatui::style::Color::Rgb(245, 158, 11);

fn three_column_line(left: &str, center: &str, right: &str, width: usize) -> Line<'static> {
    let lw = left.chars().count();
    let rw = right.chars().count();
    if width <= lw + rw + 1 {
        return Line::from(Span::raw(format!("{left}{right}")));
    }
    let mid_max = width.saturating_sub(lw + rw);
    let mut c = center.to_string();
    if c.chars().count() > mid_max {
        let take = mid_max.saturating_sub(1);
        c = c.chars().take(take).collect::<String>() + "…";
    }
    let cw = c.chars().count();
    let pad = width.saturating_sub(lw + rw + cw);
    let pad_left = pad / 2;
    let pad_right = pad - pad_left;
    Line::from(vec![
        Span::styled(
            left.to_string(),
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(pad_left), Style::default().fg(RULE)),
        Span::styled(c, Style::default().fg(ratatui::style::Color::DarkGray)),
        Span::styled(" ".repeat(pad_right), Style::default().fg(RULE)),
        Span::styled(
            right.to_string(),
            Style::default().fg(ratatui::style::Color::DarkGray),
        ),
    ])
}

/// Renders the fixed single-line header (background fill; no [`Block`] borders).
pub fn render_header_bar(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    version: &str,
    cwd: &Path,
    model_name: &str,
    provider: &str,
) {
    let inner_w = area.width as usize;
    let left = format!("▓▓ AKMON v{version}");
    let right = format!("{model_name}  │  {provider}");
    let line = three_column_line(&left, &cwd_shortened(cwd), &right, inner_w);
    f.render_widget(Paragraph::new(line).style(Style::default().bg(BG)), area);
}
