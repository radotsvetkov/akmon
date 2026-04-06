//! Bottom status strip: session, tokens, cache, cost, hint.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

const BG: ratatui::style::Color = ratatui::style::Color::Rgb(17, 17, 20);

/// Draws the session / usage / contextual hint line.
pub fn render_status_bar(f: &mut ratatui::Frame<'_>, area: Rect, parts: StatusParts) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let sep = Style::default().fg(ratatui::style::Color::Rgb(55, 62, 74));
    let dg = Style::default().fg(ratatui::style::Color::DarkGray);

    spans.push(Span::styled(parts.session_prefix.clone(), dg));
    spans.push(Span::styled("  │  ", sep));
    spans.push(Span::styled(format!("tokens:{}", parts.tokens), dg));
    spans.push(Span::styled("  │  ", sep));
    spans.push(Span::styled(
        format!("cache:{}", parts.cache),
        parts.cache_style,
    ));
    if let Some(cost) = parts.cost_line {
        spans.push(Span::styled("  │  ", sep));
        spans.push(Span::styled(cost.text, cost.style));
    }
    let left: String = spans
        .iter()
        .map(|s| s.content.as_ref().to_string())
        .collect::<Vec<_>>()
        .concat();
    let left_w = left.chars().count();
    let hint_w = parts.hint.chars().count();
    let pad = (area.width as usize).saturating_sub(left_w + hint_w);
    spans.push(Span::styled(" ".repeat(pad.min(512)), Style::default()));
    spans.push(Span::styled(
        parts.hint.clone(),
        Style::default()
            .fg(ratatui::style::Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    ));

    let block = Block::default().style(Style::default().bg(BG));
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

/// Pre-styled cost fragment when present.
pub struct CostFrag {
    /// Text such as `"~$0.04"` or `"free"`.
    pub text: String,
    /// Ratatui style (red when over threshold).
    pub style: Style,
}

/// Fields required to paint [`render_status_bar`].
pub struct StatusParts {
    /// Short session id (8 chars).
    pub session_prefix: String,
    /// Combined input + output tokens.
    pub tokens: u32,
    /// Cache read tokens.
    pub cache: u32,
    /// Style for cache field.
    pub cache_style: Style,
    /// Optional cost segment.
    pub cost_line: Option<CostFrag>,
    /// Right-aligned hint text.
    pub hint: String,
}
